use super::*;

impl Host {
    fn provider(&self, consumer: &str, service: &str, version: u32) -> Result<String, String> {
        let mut providers = Vec::new();
        for (id, entry) in &self.entries {
            let state = entry.state.lock().unwrap();
            if let Some(provided) = state
                .package
                .as_ref()
                .and_then(|p| p.manifest.provides.get(service))
            {
                providers.push((id.clone(), *provided));
            }
        }
        if providers.is_empty() {
            return Err(format!(
                "{consumer}: missing or unsupported service {service}"
            ));
        }
        if providers.len() != 1 {
            return Err(format!("{consumer}: ambiguous service {service}"));
        }
        let (id, provided) = providers.pop().unwrap();
        if provided != version {
            return Err(format!("{consumer}: service {service} version mismatch: requires {version}, provides {provided}"));
        }
        Ok(id)
    }
    fn dependencies(&self, id: &str) -> Result<Vec<String>, String> {
        let requirements = self.entries[id]
            .state
            .lock()
            .unwrap()
            .package
            .as_ref()
            .map(|p| p.manifest.requires.clone())
            .unwrap_or_default();
        requirements
            .into_iter()
            .map(|(name, version)| self.provider(id, &name, version))
            .collect()
    }
    fn visit(
        &self,
        id: &str,
        visiting: &mut Vec<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if order.iter().any(|done| done == id) {
            return Ok(());
        }
        if visiting.iter().any(|active| active == id) {
            return Err(format!("service dependency cycle at {id}"));
        }
        if !self.entries[id].state.lock().unwrap().requested_enabled {
            return Err(format!("dependency {id} is not selected"));
        }
        visiting.push(id.into());
        for dependency in self.dependencies(id)? {
            self.visit(&dependency, visiting, order)?;
        }
        visiting.pop();
        order.push(id.into());
        Ok(())
    }
    pub(crate) async fn reconcile(&self) -> Result<(), String> {
        let _cleanup = CleanupDisabled(self);
        self.reap_dependencies();
        for (id, entry) in &self.entries {
            let refresh = {
                let state = entry.state.lock().unwrap();
                state.requested_enabled && !state.enabled
            };
            if refresh {
                match Self::load_package(&self.config.join("plugins").join(id), id) {
                    Ok(package) => entry.state.lock().unwrap().package = Some(package),
                    Err(error) => entry.state.lock().unwrap().error = Some(error),
                }
            }
        }
        let mut order = Vec::new();
        let mut errors = BTreeMap::new();
        if self.optional_enabled() {
            for (id, entry) in &self.entries {
                if entry.state.lock().unwrap().requested_enabled {
                    let mut branch = Vec::new();
                    match self.visit(id, &mut Vec::new(), &mut branch) {
                        Ok(()) => {
                            for node in branch {
                                if !order.contains(&node) {
                                    order.push(node);
                                }
                            }
                        }
                        Err(error) => {
                            errors.insert(id.clone(), error);
                        }
                    }
                }
            }
        }
        // Fence every unwanted process before waiting for any in-flight RPC.
        let mut stop: Vec<String> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                if !order.contains(id) {
                    let mut state = entry.state.lock().unwrap();
                    state.enabled = false;
                    state.generation += 1;
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        self.dependents_first(&mut stop);
        for id in stop {
            self.change_enabled(&id, false).await?;
        }
        for (id, error) in errors {
            self.entries[&id].state.lock().unwrap().error = Some(error);
        }
        for id in order {
            let failed = self
                .dependencies(&id)
                .unwrap_or_default()
                .into_iter()
                .find(|dep| !self.entries[dep].state.lock().unwrap().enabled);
            if let Some(dep) = failed {
                self.change_enabled(&id, false).await?;
                self.entries[&id].state.lock().unwrap().error =
                    Some(format!("dependency {dep} is unavailable"));
            } else {
                self.change_enabled(&id, true).await?;
            }
        }
        Ok(())
    }
    pub(crate) fn dependents_first(&self, ids: &mut Vec<String>) {
        fn walk(
            host: &Host,
            id: &str,
            allowed: &[String],
            seen: &mut Vec<String>,
            out: &mut Vec<String>,
        ) {
            if seen.iter().any(|s| s == id) {
                return;
            }
            seen.push(id.into());
            for other in allowed {
                if host
                    .dependencies(other)
                    .unwrap_or_default()
                    .iter()
                    .any(|dep| dep == id)
                {
                    walk(host, other, allowed, seen, out);
                }
            }
            out.push(id.into());
        }
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for id in ids.iter() {
            walk(self, id, ids, &mut seen, &mut out);
        }
        *ids = out;
    }
    pub(crate) fn reap_dependencies(&self) {
        for entry in self.entries.values() {
            entry.reap();
        }
        let mut stopped = Vec::new();
        loop {
            let mut changed = false;
            for (id, entry) in &self.entries {
                if !entry.state.lock().unwrap().enabled {
                    continue;
                }
                let failed = self.dependencies(id).and_then(|deps| {
                    deps.into_iter()
                        .find(|dep| !self.entries[dep].state.lock().unwrap().enabled)
                        .map_or(Ok(()), |dep| {
                            Err(format!("dependency {dep} is unavailable"))
                        })
                });
                if let Err(error) = failed {
                    let mut state = entry.state.lock().unwrap();
                    state.enabled = false;
                    state.generation += 1;
                    state.error = Some(error);
                    drop(state);
                    stopped.push(id.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.dependents_first(&mut stopped);
        for id in stopped {
            if let Ok(mut slot) = self.entries[&id].process.try_lock() {
                slot.take();
            }
        }
    }
    pub(crate) async fn route_service(
        &self,
        consumer: &str,
        params: Value,
    ) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            service: String,
            version: u32,
            method: String,
            #[serde(default)]
            params: Value,
        }
        let request: Request = serde_json::from_value(params).map_err(err)?;
        if request.method.is_empty()
            || request.method.len() > 200
            || request.method.starts_with("engine.")
            || matches!(
                request.method.as_str(),
                "initialize" | "shutdown" | "services.call"
            )
        {
            return Err("invalid service method".into());
        }
        let declared = self.entries[consumer]
            .state
            .lock()
            .unwrap()
            .package
            .as_ref()
            .and_then(|p| p.manifest.requires.get(&request.service).copied());
        if declared != Some(request.version) {
            return Err("undeclared service access".into());
        }
        let provider = self.provider(consumer, &request.service, request.version)?;
        Box::pin(self.call(&provider, "services.invoke", json!({"service":request.service,"version":request.version,"method":request.method,"params":request.params}))).await
    }
}

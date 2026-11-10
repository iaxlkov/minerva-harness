//! Cordis owns plugin lifetimes; the built-in manager publishes the roster service.
mod builtin;
mod images;
use crate::providers::{ProviderPlugin, ProviderService};
use crate::shell::{ShellPlugin, ShellService};
use cordis::event::{observer_sync, ListenerRegistrationError};
#[cfg(test)]
use cordis::Routing;
use cordis::{Context, Event, FiberHandle, FiberState, Plugin, PreparedPlugin, Service};
use serde::Serialize;
use std::path::PathBuf;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

struct HealthCheck;
impl Event for HealthCheck {
    const NAME: &'static str = "studio/plugin-health-check";
    type Args = ();
    type Output = ();
}

struct ExamplePlugin {
    counter: Arc<AtomicUsize>,
}
impl Plugin for ExamplePlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = ListenerRegistrationError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let counter = self.counter.clone();
        ctx.on::<HealthCheck, _>(observer_sync(move |_, ()| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok::<_, Infallible>(())
        }))?;
        Ok(())
    }
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub requested_enabled: bool,
    pub optional_enabled: bool,
    pub can_toggle: bool,
    pub origin: String,
    pub has_ui: bool,
    pub error: Option<String>,
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginModel {
    pub id: String,
    pub name: String,
    pub group: String,
    pub is_default: bool,
}

struct ManagerService {
    optional_plugins: std::sync::Mutex<Option<Arc<dyn crate::optional::OptionalPlugins>>>,
    harness: Arc<crate::harness::Harness>,
    context: Context,
    lifecycle: Mutex<()>,
    closing: AtomicBool,
    example: Mutex<Option<FiberHandle>>,
    example_events: Arc<AtomicUsize>,
    external: Arc<minerva_plugin_host::Host>,
    tool_updates: Mutex<()>,
    lsp_updates: Mutex<()>,
    lsp_generations: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    lsp_errors: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    tool_generations: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    tool_errors: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    local: bool,
}
impl Service for ManagerService {
    const NAME: &'static str = "studio.plugin-manager";
}

impl ManagerService {
    fn optional(&self) -> Option<Arc<dyn crate::optional::OptionalPlugins>> {
        self.optional_plugins.lock().ok().and_then(|v| v.clone())
    }
    async fn sync_lsp(&self) {
        let _guard = self.lsp_updates.lock().await;
        let Ok(lsp) = self.context.try_service::<crate::lsp::LspService>() else { return; };
        let roster = self.external.roster();
        let mut desired = std::collections::BTreeMap::new();
        for entry in &roster {
            if !entry.enabled { continue; }
            if let Some(manifest) = &entry.manifest {
                for item in &manifest.language_servers {
                    let id = format!("plugin:{}/{}", entry.id, item.id);
                    desired.insert(id.clone(), (entry.id.clone(), entry.generation, crate::lsp::LanguageServer {
                        id, command: item.command.clone(), args: item.args.clone(), extensions: item.extensions.clone().into_iter().collect(),
                    }));
                }
            }
        }
        for id in lsp.external_provider_ids().await {
            let changed = match desired.get(&id) {
                Some((_, generation, provider)) => {
                    let previous = self.lsp_generations.lock().unwrap().get(&id).copied();
                    previous != Some(*generation) || lsp.provider(&id).await.as_ref() != Some(provider)
                },
                None => true,
            };
            if changed {
                lsp.unregister(&id).await;
                self.lsp_generations.lock().unwrap().remove(&id);
            }
        }
        let mut errors = std::collections::BTreeMap::new();
        for (id, (owner, generation, provider)) in desired {
            if lsp.provider(&id).await.is_none() {
                match lsp.register(provider).await {
                    Ok(()) => { self.lsp_generations.lock().unwrap().insert(id, generation); }
                    Err(error) => { errors.insert(owner, format!("LSP: {error}")); },
                }
            }
        }
        *self.lsp_errors.lock().unwrap() = errors;
    }
    async fn start_example(&self) -> Result<FiberHandle, String> {
        self.context
            .spawn(PreparedPlugin::from_input(
                ExamplePlugin {
                    counter: self.example_events.clone(),
                },
                (),
            ))
            .await
            .map_err(|error| format!("Не удалось запустить плагин: {error}"))
    }
    async fn list(&self) -> Vec<PluginInfo> {
        let example = self.example.lock().await;
        let optional_enabled = self.external.optional_enabled();
        let tool_errors = self.tool_errors.lock().unwrap().clone();
        let lsp_errors = self.lsp_errors.lock().unwrap().clone();
        let mut roster = builtin::SYSTEM
            .iter()
            .map(|plugin| PluginInfo {
                id: plugin.id.into(),
                name: plugin.name.into(),
                description: plugin.description.into(),
                enabled: (!plugin.local_only || self.local) && (plugin.active)(&self.context),
                requested_enabled: !plugin.local_only || self.local,
                optional_enabled,
                can_toggle: false,
                origin: "builtin".into(),
                has_ui: false,
                error: None,
            })
            .collect::<Vec<_>>();
        roster.push(PluginInfo {
            id: "example".into(),
            name: "Тестовый плагин".into(),
            description: "Проверяет запуск, события и отключение в Cordis.".into(),
            enabled: example
                .as_ref()
                .is_some_and(|handle| handle.state() == FiberState::Active),
            requested_enabled: self.external.example_enabled(),
            optional_enabled,
            can_toggle: true,
            origin: "builtin".into(),
            has_ui: false,
            error: None,
        });
        roster.extend(self.external.roster().into_iter().filter(|entry| !self.optional().is_some_and(|p|p.owns(&entry.id))).map(|entry| {
            if !entry.enabled {
                self.harness
                    .tools
                    .revoke_owner(&format!("plugin:{}", entry.id));
            }
            let manifest = entry.manifest;
            PluginInfo {
                name: manifest
                    .as_ref()
                    .and_then(|m| m.name.clone())
                    .unwrap_or_else(|| entry.id.clone()),
                description: manifest
                    .as_ref()
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default(),
                has_ui: manifest.as_ref().is_some_and(|m| m.ui.is_some()),
                id: entry.id.clone(),
                enabled: entry.enabled,
                requested_enabled: entry.requested_enabled,
                optional_enabled,
                can_toggle: true,
                origin: "installed".into(),
                error: entry.error.or_else(|| tool_errors.get(&entry.id).cloned()).or_else(|| lsp_errors.get(&entry.id).cloned()),
            }
        }));
        if let Some(optional) = self.optional() { roster.extend(optional.list()); }
        roster
    }
    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.closing.load(Ordering::SeqCst) {
            return Err("Plugin manager is shutting down".into());
        }
        if builtin::SYSTEM.iter().any(|plugin| plugin.id == id) {
            return Err("System plugins cannot be toggled".into());
        }
        if let Some(optional) = self.optional().filter(|p| p.owns(id)) {
            optional.set_enabled(id, enabled).await?;
            return Ok(self.list().await);
        }
        if id != "example" {
            if !enabled
                || !self
                    .external
                    .roster()
                    .iter()
                    .any(|p| p.id == id && p.enabled)
            {
                self.harness.tools.revoke_owner(&format!("plugin:{id}"));
            }
            self.external.set_enabled(id, enabled).await?;
            self.sync_lsp().await;
            return Ok(self.list().await);
        }
        self.external.set_example_enabled(enabled).await?;
        self.set_example_active(enabled && self.external.optional_enabled())
            .await?;
        Ok(self.list().await)
    }
    async fn set_optional_enabled(&self, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.closing.load(Ordering::SeqCst) {
            return Err("Plugin manager is shutting down".into());
        }
        if !enabled {
            for entry in self.external.roster() {
                self.harness
                    .tools
                    .revoke_owner(&format!("plugin:{}", entry.id));
            }
            self.tool_generations.lock().unwrap().clear();
        }
        let external = self.external.set_optional_enabled(enabled).await;
        let example = self
            .set_example_active(self.external.optional_enabled() && self.external.example_enabled())
            .await;
        external?;
        example?;
        if let Some(optional) = self.optional() { optional.set_optional_enabled(enabled).await?; }
        self.sync_lsp().await;
        Ok(self.list().await)
    }
    async fn set_example_active(&self, enabled: bool) -> Result<(), String> {
        let mut example = self.example.lock().await;
        if enabled && example.is_none() {
            *example = Some(self.start_example().await?);
        } else if !enabled {
            if let Some(handle) = example.as_ref() {
                handle
                    .dispose()
                    .await
                    .map_err(|error| format!("Не удалось отключить плагин: {error}"))?;
                *example = None;
            }
        }
        Ok(())
    }
    async fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.harness.tools.revoke_owner(images::OWNER);
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(optional) = self.optional() { let _ = optional.shutdown().await; }
        self.external.shutdown().await;
        if let Some(handle) = self.example.lock().await.take() {
            let _ = handle.dispose().await;
        }
    }
}

struct ManagerPlugin {
    service: Arc<ManagerService>,
}
impl Plugin for ManagerPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _publication = ctx.provide(self.service.clone())?;
        Ok(())
    }
}

pub struct PluginManager {
    pub harness: Arc<crate::harness::Harness>,
    context: Context,
    builtins: Vec<FiberHandle>,
    #[cfg(test)]
    example_events: Arc<AtomicUsize>,
}
impl PluginManager {
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub async fn start(settings_path: PathBuf) -> Result<Self, String> {
        Self::start_with_external_plugins(settings_path, true).await
    }
    pub async fn start_with_external_plugins(
        settings_path: PathBuf,
        local: bool,
    ) -> Result<Self, String> {
        Self::start_scoped(
            settings_path,
            local,
            crate::credential_store::CredentialScope::new("local")?,
        )
        .await
    }
    pub async fn start_scoped(
        settings_path: PathBuf,
        local: bool,
        scope: crate::credential_store::CredentialScope,
    ) -> Result<Self, String> {
        let context_path = settings_path.with_file_name("context.sqlite3");
        Self::start_scoped_at(settings_path, local, scope, context_path).await
    }
    pub async fn start_scoped_at(
        settings_path: PathBuf,
        local: bool,
        scope: crate::credential_store::CredentialScope,
        context_path: PathBuf,
    ) -> Result<Self, String> {
        let harness = Arc::new(crate::harness::Harness::prepare(&context_path, local)?);
        let external = Arc::new(minerva_plugin_host::Host::open(
            settings_path
                .parent()
                .ok_or("settings path has no parent")?
                .to_path_buf(),
            local,
        )?);
        external.restore().await?;
        // Read fallible configuration before publishing any lifecycle effects.
        let shell_service = Arc::new(ShellService::new(settings_path.clone())?);
        let provider_path = settings_path.with_file_name("providers.json");
        let provider_service = Arc::new(if local {
            ProviderService::new(provider_path)?
        } else {
            ProviderService::new_with_store(
                provider_path,
                scope,
                Arc::new(crate::credential_store::UnavailableCredentialStore),
                crate::network_policy::EndpointPolicy::Hosted,
            )?
        });
        let context = Context::new();
        let example_events = Arc::new(AtomicUsize::new(0));
        images::register(&harness.tools, external.clone(), harness.context.clone())?;
        let service = Arc::new(ManagerService {
            optional_plugins: std::sync::Mutex::new(None),
            harness: harness.clone(),
            context: context.clone(),
            lifecycle: Mutex::new(()),
            closing: AtomicBool::new(false),
            example: Mutex::new(None),
            example_events: example_events.clone(),
            external,
            tool_updates: Mutex::new(()),
            lsp_updates: Mutex::new(()),
            lsp_generations: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            lsp_errors: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            tool_generations: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            tool_errors: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            local,
        });
        let mut started = Vec::new();
        let result = async {
            started.push(
                context
                    .spawn(PreparedPlugin::from_input(
                        builtin::ToolRuntimePlugin {
                            tools: harness.tools.clone(),
                            interactions: harness.interactions.clone(),
                        },
                        (),
                    ))
                    .await
                    .map_err(|e| e.to_string())?,
            );
            started.push(
                context
                    .spawn(PreparedPlugin::from_input(
                        builtin::ContextPlugin(harness.context.clone()),
                        (),
                    ))
                    .await
                    .map_err(|e| e.to_string())?,
            );
            started.push(
                context
                    .spawn(PreparedPlugin::from_input(
                        builtin::WorkspaceToolsPlugin(local),
                        (),
                    ))
                    .await
                    .map_err(|e| e.to_string())?,
            );
            started.push(
                context
                    .spawn(PreparedPlugin::from_input(
                        builtin::McpPlugin(harness.mcp.clone()),
                        (),
                    ))
                    .await
                    .map_err(|e| e.to_string())?,
            );
            let manager = context
                .spawn(PreparedPlugin::from_input(
                    ManagerPlugin {
                        service: service.clone(),
                    },
                    (),
                ))
                .await
                .map_err(|e| e.to_string())?;
            started.push(manager.clone());
            service
                .set_example_active(
                    service.external.optional_enabled() && service.external.example_enabled(),
                )
                .await?;
            let shell = context
                .spawn(PreparedPlugin::from_input(ShellPlugin(shell_service), ()))
                .await
                .map_err(|e| e.to_string())?;
            started.push(shell.clone());
            let providers = context
                .spawn(PreparedPlugin::from_input(
                    ProviderPlugin(provider_service),
                    (),
                ))
                .await
                .map_err(|e| e.to_string())?;
            started.push(providers.clone());
            Ok::<_, String>(())
        }
        .await;
        match result {
            Ok(()) => {}
            Err(error) => {
                service.shutdown().await;
                for handle in started.iter().rev() {
                    let _ = handle.dispose().await;
                }
                return Err(error);
            }
        };
        Ok(Self {
            harness,
            context,
            builtins: started,
            #[cfg(test)]
            example_events,
        })
    }
    fn service(&self) -> Result<Arc<ManagerService>, String> {
        self.context
            .try_service::<ManagerService>()
            .map_err(|error| format!("Менеджер плагинов недоступен: {error}"))
    }
    pub async fn sync_lsp(&self) -> Result<(), String> {
        self.service()?.sync_lsp().await;
        Ok(())
    }
    pub async fn list(&self) -> Result<Vec<PluginInfo>, String> {
        Ok(self.service()?.list().await)
    }
    pub fn attach_optional(&self, plugins: Arc<dyn crate::optional::OptionalPlugins>) -> Result<(), String> {
        let service = self.service()?;
        let mut slot = service.optional_plugins.lock().map_err(|_| "optional plugin registry unavailable")?;
        if slot.is_some() { return Err("optional plugins already attached".into()); }
        if service.external.roster().iter().any(|p| p.manifest.is_some() && plugins.owns(&p.id)) { return Err("installed package conflicts with host plugin id".into()); }
        *slot = Some(plugins);
        Ok(())
    }
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        let service = self.service()?;
        let id = id.to_owned();
        // Accepted lifecycle changes finish even if their transport disconnects.
        tokio::spawn(async move { service.set_enabled(&id, enabled).await })
            .await
            .map_err(|e| e.to_string())?
    }
    pub async fn set_optional_enabled(&self, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        let service = self.service()?;
        tokio::spawn(async move { service.set_optional_enabled(enabled).await })
            .await
            .map_err(|e| e.to_string())?
    }
    pub async fn plugin_call(
        &self,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if let Some(optional) = self.service()?.optional().filter(|p| p.owns(id)) {
            return optional.call(id, method, params).await;
        }
        if method.starts_with("engine.") || (method.starts_with("tools.") && method != "tools.list")
        {
            return Err("Engine methods require a supervised run".into());
        }
        self.service()?.external.call(id, method, params).await
    }
    pub(crate) async fn engine_call(
        &self,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.engine_host(id)?.call(id, method, params).await
    }
    pub(crate) fn engine_host(&self, id: &str) -> Result<Arc<minerva_plugin_host::Host>, String> {
        let host = self.service()?.external.clone();
        if !host.roster().iter().any(|entry| {
            entry.enabled
                && entry.id == id
                && entry
                    .manifest
                    .as_ref()
                    .is_some_and(|m| m.capabilities.iter().any(|c| c == "model-engine"))
        }) {
            return Err("Model engine is not enabled".into());
        }
        Ok(host)
    }
    pub async fn models(&self) -> Result<Vec<PluginModel>, String> {
        let service = self.service()?;
        let mut requests = tokio::task::JoinSet::new();
        for entry in service.external.roster() {
            let Some(manifest) = entry.manifest else {
                continue;
            };
            if !entry.enabled || !manifest.capabilities.iter().any(|c| c == "model-engine") {
                continue;
            }
            let generation = entry.generation;
            let service = service.clone();
            requests.spawn(async move {
                let result = service
                    .external
                    .call(&manifest.id, "engine.models", serde_json::json!({}))
                    .await;
                (manifest, generation, result)
            });
        }
        let mut models = Vec::new();
        while let Some(result) = requests.join_next().await {
            let (manifest, generation, response) = result.map_err(|e| e.to_string())?;
            match response.and_then(|value| catalogue(&manifest, value)) {
                Ok(mut catalogue) => {
                    service
                        .external
                        .set_catalog_error(&manifest.id, generation, None);
                    models.append(&mut catalogue);
                }
                Err(error) => {
                    service
                        .external
                        .set_catalog_error(&manifest.id, generation, Some(error))
                }
            }
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }
    pub async fn ui_document(&self, id: &str) -> Result<String, String> {
        self.service()?.external.ui_document(id)
    }
    pub fn shell(&self) -> Result<Arc<ShellService>, String> {
        self.context
            .try_service::<ShellService>()
            .map_err(|error| format!("Оболочка недоступна: {error}"))
    }
    pub fn providers(&self) -> Result<Arc<ProviderService>, String> {
        self.context
            .try_service::<ProviderService>()
            .map_err(|e| e.to_string())
    }
    pub async fn shutdown(&self) {
        if let Ok(service) = self.service() {
            service.shutdown().await;
        }
        for plugin in self.builtins.iter().rev() {
            let _ = plugin.dispose().await;
        }
    }
    #[cfg(test)]
    async fn probe(&self) -> usize {
        self.context
            .emit::<HealthCheck>(Routing::Unscoped, ())
            .await
            .unwrap();
        self.example_events.load(Ordering::Relaxed)
    }
}

impl PluginManager {
    pub async fn tools_for(&self, session: &str) -> Result<crate::tools::AdmittedTools, String> {
        use sha2::{Digest, Sha256};
        let service = self.service()?;
        service.sync_lsp().await;
        let _update = service.tool_updates.lock().await;
        for entry in service.external.roster() {
            let owner = format!("plugin:{}", entry.id);
            if !entry.enabled {
                service.harness.tools.revoke_owner(&owner);
                service.tool_generations.lock().unwrap().remove(&entry.id);
                continue;
            }
            if !entry
                .manifest
                .as_ref()
                .is_some_and(|m| m.capabilities.iter().any(|c| c == "tools"))
            {
                continue;
            }
            if service.tool_generations.lock().unwrap().get(&entry.id) == Some(&entry.generation) {
                continue;
            }
            service.harness.tools.revoke_owner(&owner);
            let response = service
                .external
                .call(&entry.id, "tools.list", serde_json::json!({}))
                .await;
            let result = async {
                let definitions: Vec<crate::tools::ToolDefinition> =
                    serde_json::from_value(response?).map_err(|e| e.to_string())?;
                if definitions.len() > 256 {
                    return Err("Plugin tool catalogue exceeds limit".into());
                }
                for mut definition in definitions {
                    let raw_name = definition.name.clone();
                    if entry.manifest.as_ref().is_some_and(images::is_generator)
                        && matches!(raw_name.as_str(), "generate_image" | "image_status")
                    {
                        continue;
                    }
                    if raw_name.is_empty()
                        || raw_name.len() > 64
                        || !raw_name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        return Err("Invalid plugin tool name".into());
                    }
                    let hash = format!("{:x}", Sha256::digest(entry.id.as_bytes()));
                    definition.name = format!("plugin_{}__{raw_name}", &hash[..16]);
                    let host = service.external.clone();
                    let plugin = entry.id.clone();
                    service
                        .harness
                        .tools
                        .register(&owner, definition, move |context, args| {
                            let host = host.clone();
                            let plugin = plugin.clone();
                            let name = raw_name.clone();
                            Box::pin(async move {
                                run_plugin_tool(host, &plugin, &name, args, context).await
                            })
                        })?;
                }
                Ok::<(), String>(())
            }
            .await;
            match result {
                Ok(()) => {
                    service
                        .tool_generations
                        .lock()
                        .unwrap()
                        .insert(entry.id.clone(), entry.generation);
                    service.tool_errors.lock().unwrap().remove(&entry.id);
                }
                Err(error) => {
                    service.harness.tools.revoke_owner(&owner);
                    service.tool_errors.lock().unwrap().insert(entry.id, error);
                }
            }
        }
        let admitted = self.harness.tools.snapshot();
        let admitted = if images::has_generators(&service.external) {
            admitted
        } else {
            admitted.without_owner(images::OWNER)
        };
        let settings = self.harness.context.workspace_get(session)?;
        Ok(if self.harness.local && settings.cwd.is_some() {
            admitted
        } else {
            admitted.without_workspace()
        })
    }
    pub async fn compact_messages(
        &self,
        session: &str,
        model: &str,
        messages: &[crate::sessions::SessionMessage],
    ) -> Result<crate::context::ContextSummary, String> {
        let (_sender, mut cancel) = tokio::sync::watch::channel(false);
        self.compact_messages_cancellable(session, model, messages, &mut cancel)
            .await
    }
    pub async fn compact_messages_cancellable(
        &self,
        session: &str,
        model: &str,
        messages: &[crate::sessions::SessionMessage],
        cancel: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<crate::context::ContextSummary, String> {
        if *cancel.borrow() {
            return Err("Run cancelled".into());
        }
        let prefix = crate::context::compact_prefix(messages, 4);
        if prefix.is_empty() {
            return Err("There are not enough complete turns to compact".into());
        }
        let content = serde_json::to_string(
            &prefix
                .iter()
                .map(|m| serde_json::json!({"role":m.role,"parts":m.parts}))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
        let summary = if let Some(route) = model.strip_prefix("api:") {
            let (provider, model) = route.split_once('/').ok_or("Invalid model route")?;
            let providers = self.providers()?;
            tokio::select! {
                biased;
                _=cancel.changed()=>return Err("Run cancelled".into()),
                summary=providers.summarize(provider,model,&content)=>summary?,
            }
        } else {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let run = format!("summary:{session}:{}", NEXT.fetch_add(1, Ordering::SeqCst));
            let prompt=format!("Summarize this earlier conversation for continuation. Preserve decisions, changed files, tool outcomes and open tasks. Do not execute tools. Return only the summary.\n{content}");
            crate::engine::run(
                self,
                &run,
                model,
                vec![crate::sessions::Turn::plain("user", &prompt)],
                None,
                Arc::new(|_| Ok(())),
                cancel,
            )
            .await?
            .0
            .text()
        };
        if *cancel.borrow() {
            return Err("Run cancelled".into());
        }
        self.harness
            .context
            .save_summary(session, &prefix, &summary)
    }
}

// Dropping an HTTP request or model future must still stop its external work.
pub(crate) struct ProcessRunLease {
    host: Arc<minerva_plugin_host::Host>,
    plugin: String,
    prefix: &'static str,
    reference: serde_json::Value,
    armed: bool,
}
impl ProcessRunLease {
    pub(crate) fn new(
        host: Arc<minerva_plugin_host::Host>,
        plugin: &str,
        prefix: &'static str,
        reference: serde_json::Value,
    ) -> Self {
        Self {
            host,
            plugin: plugin.into(),
            prefix,
            reference,
            armed: true,
        }
    }
    pub(crate) async fn finish(&mut self, failed: bool) {
        if failed {
            let _ = self
                .host
                .call(
                    &self.plugin,
                    &format!("{}.cancel", self.prefix),
                    self.reference.clone(),
                )
                .await;
        }
        let _ = self
            .host
            .call(
                &self.plugin,
                &format!("{}.release", self.prefix),
                self.reference.clone(),
            )
            .await;
        self.armed = false;
    }
}
impl Drop for ProcessRunLease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let host = self.host.clone();
            let plugin = self.plugin.clone();
            let prefix = self.prefix;
            let reference = self.reference.clone();
            runtime.spawn(async move {
                let _ = host
                    .call(&plugin, &format!("{prefix}.cancel"), reference.clone())
                    .await;
                let _ = host
                    .call(&plugin, &format!("{prefix}.release"), reference)
                    .await;
            });
        }
    }
}

async fn run_plugin_tool(
    host: Arc<minerva_plugin_host::Host>,
    plugin: &str,
    name: &str,
    args: serde_json::Value,
    context: crate::tools::ToolContext,
) -> Result<serde_json::Value, String> {
    let wire_run = serde_json::to_string(&(&context.session_id, &context.run_id))
        .map_err(|e| e.to_string())?;
    let reference = serde_json::json!({"runId":wire_run,"callId":context.call_id});
    let mut lease = ProcessRunLease::new(host.clone(), plugin, "tools", reference.clone());
    let result=async {
        host.call(plugin,"tools.start",serde_json::json!({"runId":wire_run,"callId":context.call_id,"name":name,"arguments":args})).await?;
        loop {
            if context.cancellation.is_cancelled(){return Err("Tool cancelled".into());}
            let snapshot=host.call(plugin,"tools.poll",reference.clone()).await?;
            match snapshot["status"].as_str() {
                Some("completed")=>return Ok(snapshot["output"].clone()),
                Some("failed"|"cancelled")=>return Err(snapshot["error"].as_str().unwrap_or("Plugin tool failed").into()),
                Some("running")=>{},_=>return Err("Invalid plugin tool snapshot".into()),
            }
            tokio::select!{_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{},_=context.cancellation.cancelled()=>{}}
        }
    }.await;
    lease.finish(result.is_err()).await;
    result
}

fn catalogue(
    manifest: &minerva_plugin_sdk::Manifest,
    value: serde_json::Value,
) -> Result<Vec<PluginModel>, String> {
    let catalogue: minerva_plugin_sdk::engine::EngineCatalog =
        serde_json::from_value(value).map_err(|e| format!("Invalid engine catalogue: {e}"))?;
    if let Some(error) = catalogue.error.filter(|error| !error.is_empty()) {
        return Err(error);
    }
    if catalogue.models.len() > 1000 {
        return Err("Engine catalogue exceeds limit".into());
    }
    let mut ids = std::collections::HashSet::new();
    catalogue
        .models
        .into_iter()
        .map(|model| {
            if model.id.is_empty()
                || model.id.len() > 256
                || model.id.chars().any(char::is_control)
                || !ids.insert(model.id.clone())
            {
                return Err("Invalid or duplicate engine model ID".into());
            }
            Ok(PluginModel {
                id: format!("plugin:{}/{}", manifest.id, model.id),
                name: model.name,
                group: manifest.name.clone().unwrap_or_else(|| manifest.id.clone()),
                is_default: model.is_default,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn accepted_optional_toggle_finishes_after_caller_disconnects() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(
            PluginManager::start(temp.path().join("shell.json"))
                .await
                .unwrap(),
        );
        let service = manager.service().unwrap();
        let guard = service.lifecycle.lock().await;
        let references = Arc::strong_count(&service);
        let requester = manager.clone();
        let request = tokio::spawn(async move { requester.set_optional_enabled(false).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while Arc::strong_count(&service) == references {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        request.abort();
        let _ = request.await;
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let roster = manager.list().await.unwrap();
                if roster
                    .iter()
                    .all(|p| !p.optional_enabled && (!p.can_toggle || !p.enabled))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(manager.probe().await, 0);
        assert!(manager.providers().is_ok());
        assert!(manager.shell().is_ok());
        manager.shutdown().await;
        assert!(service.set_optional_enabled(true).await.is_err());
    }
    #[tokio::test]
    async fn optional_switch_preserves_system_plugins_and_selection_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("shell.json");
        let manager = PluginManager::start(path.clone()).await.unwrap();
        manager.set_optional_enabled(false).await.unwrap();
        assert_eq!(manager.probe().await, 0);
        for entry in manager.list().await.unwrap() {
            assert!(!entry.optional_enabled);
            if entry.can_toggle {
                assert!(!entry.enabled);
                assert!(entry.requested_enabled);
            } else {
                // This harness-only fixture does not compose session execution.
                assert_eq!(
                    entry.enabled,
                    !["sessions", "message-queue", "agent-loop", "workspaces", "mh-editor", "mh-lsp", "mh-lsp-rust"].contains(&entry.id.as_str())
                );
                assert!(manager.set_enabled(&entry.id, false).await.is_err());
            }
        }
        manager.shutdown().await;
        let manager = PluginManager::start(path.clone()).await.unwrap();
        assert_eq!(manager.probe().await, 0);
        manager.set_enabled("example", false).await.unwrap();
        manager.set_optional_enabled(true).await.unwrap();
        assert_eq!(manager.probe().await, 0);
        manager.set_enabled("example", true).await.unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.set_optional_enabled(false).await.unwrap();
        manager.set_optional_enabled(true).await.unwrap();
        assert_eq!(manager.probe().await, 2);
        manager.shutdown().await;
        let manager = PluginManager::start(path).await.unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.shutdown().await;
    }
    #[test]
    fn catalogue_rejects_entire_bad_response_without_partial_models() {
        use serde_json::json;
        let manifest: minerva_plugin_sdk::Manifest = serde_json::from_value(json!({
            "id":"test.engine", "version":"1", "apiVersion":1,
            "backend":{"kind":"process","entry":"engine"}
        }))
        .unwrap();
        let model = json!({"id":"good","name":"Good","isDefault":true});
        let valid = json!({"models":[model.clone()],"loading":false,"error":null});
        assert_eq!(
            catalogue(&manifest, valid).unwrap()[0].id,
            "plugin:test.engine/good"
        );
        for invalid in [
            json!({"models":[model.clone(),{"id":"","name":"Bad","isDefault":false}],"loading":false,"error":null}),
            json!({"models":[model.clone(),model.clone()],"loading":false,"error":null}),
            json!({"models":vec![model;1001],"loading":false,"error":null}),
            json!({"models":[],"loading":false,"error":"Account unavailable"}),
            json!("broken"),
        ] {
            assert!(catalogue(&manifest, invalid).is_err());
        }
    }
    #[tokio::test]
    async fn plugin_manager_publishes_service_and_cleans_up_lifetimes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = PluginManager::start(temp.path().join("shell.json"))
            .await
            .unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.set_enabled("example", false).await.unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.set_enabled("example", true).await.unwrap();
        assert_eq!(manager.probe().await, 2);
        assert!(manager.set_enabled("plugin-manager", false).await.is_err());
        manager.shutdown().await;
        assert_eq!(manager.probe().await, 2);
        assert!(manager.service().is_err());
    }
}

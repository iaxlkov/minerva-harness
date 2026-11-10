//! Host-scoped experiment operations. No model run is admitted without an isolated child-run port.
use crate::learning::Learning;
use minerva_plugin_sdk::learning::{AccessContext, Scope};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

use mh_agent_loop::AgentLoopService;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};

pub(crate) struct ExecutionSupervisor {
    jobs: Mutex<BTreeMap<String, RunningJob>>,
    permits: Arc<tokio::sync::Semaphore>,
}
impl Default for ExecutionSupervisor {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(BTreeMap::new()),
            permits: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}
async fn acquire_permit(
    supervisor: &ExecutionSupervisor,
    mut cancel: watch::Receiver<bool>,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    if *cancel.borrow() {
        return Err("cancelled".into());
    }
    tokio::select! { biased; _=cancel.changed()=>Err("cancelled".into()), permit=supervisor.permits.clone().acquire_owned()=>permit.map_err(|e|e.to_string()) }
}
fn prune_jobs(jobs: &mut BTreeMap<String, RunningJob>) {
    jobs.retain(|_, job| job.task.as_ref().is_none_or(|task| !task.is_finished()));
}

impl ExecutionSupervisor {
    pub(crate) async fn register_custom(
        &self,
        access: &AccessContext,
        scope: &Scope,
        job: &str,
        cancel: watch::Sender<bool>,
        task: JoinHandle<()>,
    ) -> Result<(), String> {
        access.authorize(scope).map_err(|e| e.to_string())?;
        let id = key(access, scope, job)?;
        let mut jobs = self.jobs.lock().await;
        prune_jobs(&mut jobs);
        if jobs.len() >= 256 {
            return Err("LIMIT".into());
        }
        if jobs.contains_key(&id) {
            return Err("CONFLICT".into());
        }
        jobs.insert(
            id,
            RunningJob {
                cancel,
                children: Vec::new(),
                task: Some(task),
            },
        );
        Ok(())
    }
    pub(crate) async fn cancel_custom(
        &self,
        access: &AccessContext,
        scope: &Scope,
        job: &str,
        service: &AgentLoopService,
    ) -> Result<(), String> {
        access.authorize(scope).map_err(|e| e.to_string())?;
        let id = key(access, scope, job)?;
        let task = {
            let mut jobs = self.jobs.lock().await;
            let running = jobs.get_mut(&id).ok_or("NOT_FOUND")?;
            running.cancel.send_replace(true);
            for (session, run) in &running.children {
                let _ = service.cancel_run(session, run);
            }
            running.task.take()
        };
        if let Some(task) = task {
            task.await.map_err(|e| e.to_string())?;
        }
        self.jobs.lock().await.remove(&id);
        Ok(())
    }
    pub(crate) async fn shutdown(&self, service: &AgentLoopService) {
        let tasks = {
            let mut jobs = self.jobs.lock().await;
            let mut tasks = Vec::new();
            for running in jobs.values_mut() {
                running.cancel.send_replace(true);
                for (session, run) in &running.children {
                    let _ = service.cancel_run(session, run);
                }
                if let Some(task) = running.task.take() {
                    tasks.push(task);
                }
            }
            jobs.clear();
            tasks
        };
        for task in tasks {
            let _ = task.await;
        }
    }
}
struct RunningJob {
    cancel: watch::Sender<bool>,
    children: Vec<(String, String)>,
    task: Option<JoinHandle<()>>,
}
fn key(access: &AccessContext, scope: &Scope, job: &str) -> Result<String, String> {
    serde_json::to_string(&(&access.actor_id, scope, job)).map_err(|e| e.to_string())
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str, String> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("INVALID_INPUT: {key}"))
}
fn encode<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
fn error(e: mh_experiments::Error) -> String {
    format!("{e:?}")
}
struct DraftRunner;
impl mh_experiments::HostRunner for DraftRunner {
    fn start(&self, _: &str, _: &str) -> std::result::Result<(), mh_experiments::Error> {
        Err(mh_experiments::Error::EnvironmentUnavailable)
    }
    fn cancel_and_join(&self, _: &str) -> std::result::Result<(), mh_experiments::Error> {
        Ok(())
    }
}
impl Learning {
    async fn model_descriptor(&self, model: &str) -> Result<Value, String> {
        let (provider, model_name) = model
            .strip_prefix("api:")
            .and_then(|v| v.split_once('/'))
            .ok_or("UNSUPPORTED_ENGINE")?;
        let manager = self.manager.upgrade().ok_or("plugin manager unavailable")?;
        let providers =
            serde_json::to_value(manager.providers()?.list().await).map_err(|e| e.to_string())?;
        let mut entry = providers
            .as_array()
            .and_then(|p| p.iter().find(|p| p["id"] == provider))
            .cloned()
            .ok_or("provider unavailable")?;
        if !entry["models"]
            .as_array()
            .is_some_and(|models| models.iter().any(|v| v == model_name))
        {
            return Err("model unavailable".into());
        }
        if let Some(object) = entry.as_object_mut() {
            object.remove("hasKey");
        }
        Ok(entry)
    }
    pub(crate) async fn execution_call(
        &self,
        id: &str,
        method: &str,
        access: &AccessContext,
        scope: &Scope,
        payload: Value,
    ) -> Result<Value, String> {
        access.authorize(scope).map_err(|e| e.to_string())?;
        match (id, method) {
            ("experiments", "list") | ("improvement", "list") => {
                encode(self.experiments.host_list(access, scope).map_err(error)?)
            }
            ("experiments", "get") | ("improvement", "get") => encode(
                self.experiments
                    .host_get(access, scope, text(&payload, "id")?)
                    .map_err(error)?,
            ),
            ("experiments", "create") => {
                let task = text(&payload, "task")?;
                let model = text(&payload, "model")?;
                if task.len() > 16384 || model.len() > 256 {
                    return Err("INVALID_INPUT".into());
                }
                let budget = payload
                    .get("budget")
                    .and_then(Value::as_i64)
                    .filter(|n| (1..=1000).contains(n))
                    .ok_or("INVALID_INPUT: budget")?;
                let k = payload.get("k").and_then(Value::as_u64).unwrap_or(1);
                if k != 1 {
                    return Err("UNSUPPORTED: only k=1 is currently available".into());
                }
                let frozen = self
                    .context
                    .prepare_enriched(&access.session_id, &[])
                    .await?
                    .system_prompt;
                let spec =
                    json!({"task":task,"model":model,"budget":budget,"k":k,"frozenContext":frozen});
                let bytes = serde_json::to_vec(&spec).map_err(|e| e.to_string())?;
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let job = format!(
                    "experiment-{stamp}-{}",
                    SEQUENCE.fetch_add(1, Ordering::Relaxed)
                );
                let limits = mh_experiments::Limits {
                    max_calls: budget,
                    max_input: 1_000_000,
                    max_output: 100_000,
                    max_concurrency: 1,
                    max_wall_ms: 3_600_000,
                    max_depth: 1,
                    max_cost_minor: None,
                    input_rate_per_million: None,
                    output_rate_per_million: None,
                };
                self.experiments
                    .host_create(access, scope, &job, &digest, &limits)
                    .map_err(error)?;
                let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
                self.experiments
                    .put_artifact(&access.actor_id, &scope_key, &job, &bytes)
                    .map_err(error)?;
                encode(
                    self.experiments
                        .host_get(access, scope, &job)
                        .map_err(error)?,
                )
            }
            ("improvement", "start") => {
                let task = text(&payload, "task")?.to_owned();
                let model = text(&payload, "model")?.to_owned();
                let k = payload.get("k").and_then(Value::as_u64).unwrap_or(1);
                if !(1..=5).contains(&k) {
                    return Err("INVALID_INPUT: k".into());
                }
                let mode = match payload
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("sequential")
                {
                    "parallel" => mh_improvement::Mode::Parallel,
                    "sequential" => mh_improvement::Mode::Sequential,
                    _ => return Err("INVALID_INPUT: mode".into()),
                };
                let required = 7 * k + 1 + u64::from(k > 1);
                let budget = payload
                    .get("budget")
                    .and_then(Value::as_i64)
                    .filter(|v| *v >= required as i64 && *v <= 1000)
                    .ok_or("INVALID_INPUT: insufficient stage budget")?;
                if task.len() > 16384 || !model.starts_with("api:") {
                    return Err("UNSUPPORTED_ENGINE or invalid task".into());
                }
                let provider_snapshot = self.model_descriptor(&model).await?;
                let mut frozen = self
                    .context
                    .prepare_enriched(&access.session_id, &[])
                    .await?
                    .system_prompt;
                let mut library = Vec::new();
                let task_words: std::collections::BTreeSet<String> = task
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| s.chars().count() >= 4)
                    .map(str::to_lowercase)
                    .collect();
                for index in self
                    .experience
                    .search(access, scope, "", 50)
                    .map_err(|e| e.to_string())?
                {
                    let item = self
                        .experience
                        .get(access, scope, &index.id)
                        .map_err(|e| e.to_string())?;
                    let search =
                        format!("{} {} {}", index.cue, index.domain, item.publication.lesson)
                            .to_lowercase();
                    if task_words.is_empty()
                        || !search
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|word| task_words.contains(word))
                    {
                        continue;
                    }
                    let section = format!(
                        "\nRelevant validated experience ({}): {}\n",
                        index.id, item.publication.lesson
                    );
                    if index.status == minerva_plugin_sdk::experience::ExperienceStatus::Validated
                        && frozen.len() + section.len() <= 240_000
                    {
                        frozen.push_str(&section);
                    }
                    library.push(json!({"id":index.id,"revision":index.revision,"bundleDigest":index.bundle_digest,"status":index.status,"cue":index.cue,"lesson":item.publication.lesson,"limitations":item.publication.limitations,"evidenceRefs":item.publication.evidence_refs}));
                    if library.len() == 8 {
                        break;
                    }
                }
                let library_revision = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&library).map_err(|e| e.to_string())?)
                );
                let base = BTreeMap::new();
                let (_, base_digest) = materialize_bundle(&base, &[])?;
                let spec = json!({"task":task,"model":model,"budget":budget,"k":k,"mode":mode,"frozenContext":frozen,"bundleDigest":base_digest,"library":library,"providerSnapshot":provider_snapshot});
                let bytes = serde_json::to_vec(&spec).map_err(|e| e.to_string())?;
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let job = format!(
                    "improvement-{stamp}-{}",
                    SEQUENCE.fetch_add(1, Ordering::Relaxed)
                );
                let limits = mh_experiments::Limits {
                    max_calls: budget,
                    max_input: 100_000 * required as i64,
                    max_output: 4096 * required as i64,
                    max_concurrency: 1,
                    max_wall_ms: 3_600_000,
                    max_depth: 1,
                    max_cost_minor: None,
                    input_rate_per_million: None,
                    output_rate_per_million: None,
                };
                self.experiments
                    .host_create(access, scope, &job, &digest, &limits)
                    .map_err(error)?;
                let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
                self.experiments
                    .put_artifact(&access.actor_id, &scope_key, &job, &bytes)
                    .map_err(error)?;
                self.experiments
                    .host_add_trial(access, scope, &job, "trial", &digest)
                    .map_err(error)?;
                let generation = self
                    .experiments
                    .host_start(access, scope, &job)
                    .map_err(error)?;
                let job_key = key(access, scope, &job)?;
                let mut jobs = self.execution.jobs.lock().await;
                prune_jobs(&mut jobs);
                if jobs.len() >= 256 {
                    let _ = self
                        .experiments
                        .host_finish(access, scope, &job, generation, "failed");
                    return Err("LIMIT".into());
                }
                let (cancel, receiver) = watch::channel(false);
                let host = self.myself()?;
                let run_access = access.clone();
                let run_scope = scope.clone();
                let run_job = job.clone();
                let worker = tokio::spawn(async move {
                    let handle = tokio::runtime::Handle::current();
                    let work_host = host.clone();
                    let worker_job = run_job.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        run_improvement(
                            work_host,
                            run_access,
                            run_scope,
                            worker_job,
                            generation,
                            task,
                            model,
                            frozen,
                            base_digest,
                            library_revision,
                            library,
                            budget,
                            k as u8,
                            mode,
                            None,
                            receiver,
                            handle,
                        )
                    })
                    .await;
                    if let Err(e) = outcome {
                        eprintln!("improvement worker failed: {e}");
                    }
                });
                jobs.insert(
                    job_key,
                    RunningJob {
                        cancel,
                        children: Vec::new(),
                        task: Some(worker),
                    },
                );
                drop(jobs);
                encode(
                    self.experiments
                        .host_get(access, scope, &job)
                        .map_err(error)?,
                )
            }
            ("experiments", "start") => {
                let job = text(&payload, "id")?.to_owned();
                let view = self
                    .experiments
                    .host_get(access, scope, &job)
                    .map_err(error)?;
                if view.state != "draft" {
                    return Err("CONFLICT".into());
                }
                let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
                let bytes = self
                    .experiments
                    .artifact(&access.actor_id, &scope_key, &job, &view.input_digest)
                    .map_err(error)?;
                let spec: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                let task = text(&spec, "task")?.to_owned();
                let frozen = text(&spec, "frozenContext")?.to_owned();
                let model = text(&spec, "model")?.to_owned();
                if !model.starts_with("api:") {
                    return Err("UNSUPPORTED_ENGINE".into());
                }
                let manager = self.manager.upgrade().ok_or("plugin manager unavailable")?;
                let loop_service = manager
                    .context()
                    .try_service::<AgentLoopService>()
                    .map_err(|e| e.to_string())?;
                let job_key = key(access, scope, &job)?;
                let mut running = self.execution.jobs.lock().await;
                prune_jobs(&mut running);
                if running.len() >= 256 {
                    return Err("LIMIT".into());
                }
                if running.contains_key(&job_key) {
                    return Err("CONFLICT".into());
                }
                let session = format!("{job}-child");
                self.sessions
                    .create_internal(&session, "experiment", &job)?;
                self.context.freeze_context(&session, &frozen)?;
                self.experiments
                    .host_add_trial(access, scope, &job, "trial", &view.input_digest)
                    .map_err(error)?;
                let generation = self
                    .experiments
                    .host_start(access, scope, &job)
                    .map_err(error)?;
                let (cancel, receiver) = watch::channel(false);
                let supervisor = self.execution.clone();
                let store = self.experiments.clone();
                let run_access = access.clone();
                let run_scope = scope.clone();
                let spawned_key = job_key.clone();
                let run_job = job.clone();
                let task_handle = tokio::spawn(async move {
                    run_baseline(
                        supervisor,
                        store,
                        loop_service,
                        run_access,
                        run_scope,
                        spawned_key,
                        run_job,
                        session,
                        model,
                        task,
                        generation,
                        receiver,
                    )
                    .await;
                });
                running.insert(
                    job_key,
                    RunningJob {
                        cancel,
                        children: Vec::new(),
                        task: Some(task_handle),
                    },
                );
                drop(running);
                encode(
                    self.experiments
                        .host_get(access, scope, &job)
                        .map_err(error)?,
                )
            }
            ("experiments", "cancel") | ("improvement", "cancel") => {
                let job = text(&payload, "id")?;
                let job_key = key(access, scope, job)?;
                let manager = self.manager.upgrade().ok_or("plugin manager unavailable")?;
                let service = manager
                    .context()
                    .try_service::<AgentLoopService>()
                    .map_err(|e| e.to_string())?;
                let task = {
                    let mut jobs = self.execution.jobs.lock().await;
                    if let Some(r) = jobs.get_mut(&job_key) {
                        r.cancel.send_replace(true);
                        for (session, run) in &r.children {
                            let _ = service.cancel_run(session, run);
                        }
                        r.task.take()
                    } else {
                        None
                    }
                };
                if let Some(task) = task {
                    task.await.map_err(|e| e.to_string())?;
                }
                let view = self
                    .experiments
                    .host_get(access, scope, job)
                    .map_err(error)?;
                if matches!(
                    view.state.as_str(),
                    "draft" | "running" | "cancelling" | "cancelled"
                ) {
                    self.experiments
                        .host_cancel(access, scope, job, &DraftRunner)
                        .map_err(error)?;
                }
                self.execution.jobs.lock().await.remove(&job_key);
                Ok(Value::Null)
            }
            ("experiments", "compare") | ("improvement", "compare") => {
                let job = text(&payload, "id")?;
                let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
                self.experiments
                    .host_get(access, scope, job)
                    .map_err(error)?;
                let checkpoint = self
                    .experiments
                    .load_checkpoint(&access.actor_id, &scope_key, job)
                    .map_err(error)?
                    .ok_or("NOT_FOUND")?;
                serde_json::from_str(&checkpoint.1).map_err(|e| e.to_string())
            }
            ("improvement", "resume") => {
                let job = text(&payload, "id")?.to_owned();
                let view = self
                    .experiments
                    .host_get(access, scope, &job)
                    .map_err(error)?;
                if view.state != "interrupted" {
                    return Err("CONFLICT".into());
                }
                let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
                let spec_bytes = self
                    .experiments
                    .artifact(&access.actor_id, &scope_key, &job, &view.input_digest)
                    .map_err(error)?;
                let spec: Value = serde_json::from_slice(&spec_bytes).map_err(|e| e.to_string())?;
                let task = text(&spec, "task")?.to_owned();
                let model = text(&spec, "model")?.to_owned();
                let frozen = text(&spec, "frozenContext")?.to_owned();
                let base = text(&spec, "bundleDigest")?.to_owned();
                if self.model_descriptor(&model).await? != spec["providerSnapshot"] {
                    return Err("ENVIRONMENT_CHANGED".into());
                }
                let k = spec["k"]
                    .as_u64()
                    .filter(|k| (1..=5).contains(k))
                    .ok_or("invalid spec")? as u8;
                let mode: mh_improvement::Mode =
                    serde_json::from_value(spec["mode"].clone()).map_err(|e| e.to_string())?;
                let budget = spec["budget"].as_i64().ok_or("invalid budget")?;
                let library = spec["library"]
                    .as_array()
                    .cloned()
                    .ok_or("invalid library")?;
                let revision = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&library).map_err(|e| e.to_string())?)
                );
                let (seq, checkpoint) = self
                    .experiments
                    .load_checkpoint(&access.actor_id, &scope_key, &job)
                    .map_err(error)?
                    .ok_or("NOT_FOUND")?;
                let value: Value = serde_json::from_str(&checkpoint).map_err(|e| e.to_string())?;
                if value["pending"] != false {
                    return Err("UNCERTAIN_STAGE: reconciliation required".into());
                }
                let request = mh_improvement::Request {
                    mode: mode.clone(),
                    k,
                    budget: budget as u32,
                    input_digest: base.clone(),
                    library_revision: revision.clone(),
                    allowed_components: vec![
                        "systemPrompt".into(),
                        "toolGuidance".into(),
                        "memory".into(),
                        "skills".into(),
                    ],
                };
                if value["request"] != serde_json::to_value(&request).map_err(|e| e.to_string())? {
                    return Err("ENVIRONMENT_CHANGED".into());
                }
                let mut journal: mh_improvement::Journal =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
                journal.resume(&base, &revision)?;
                let job_key = key(access, scope, &job)?;
                let mut jobs = self.execution.jobs.lock().await;
                prune_jobs(&mut jobs);
                if jobs.len() >= 256 || jobs.contains_key(&job_key) {
                    return Err("CONFLICT".into());
                }
                let generation = self
                    .experiments
                    .host_resume(access, scope, &job, &view.input_digest)
                    .map_err(error)?;
                let (cancel, receiver) = watch::channel(false);
                let host = self.myself()?;
                let run_access = access.clone();
                let run_scope = scope.clone();
                let run_job = job.clone();
                let worker = tokio::spawn(async move {
                    let handle = tokio::runtime::Handle::current();
                    let _ = tokio::task::spawn_blocking(move || {
                        run_improvement(
                            host,
                            run_access,
                            run_scope,
                            run_job,
                            generation,
                            task,
                            model,
                            frozen,
                            base,
                            revision,
                            library,
                            budget,
                            k,
                            mode,
                            Some((seq, checkpoint)),
                            receiver,
                            handle,
                        )
                    })
                    .await;
                });
                jobs.insert(
                    job_key,
                    RunningJob {
                        cancel,
                        children: Vec::new(),
                        task: Some(worker),
                    },
                );
                drop(jobs);
                encode(
                    self.experiments
                        .host_get(access, scope, &job)
                        .map_err(error)?,
                )
            }
            ("experiments", "resume") => Err("UNCERTAIN_STAGE: reconciliation required".into()),
            _ => Err("unknown execution method".into()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_baseline(
    supervisor: Arc<ExecutionSupervisor>,
    store: Arc<mh_experiments::Store>,
    service: Arc<AgentLoopService>,
    access: AccessContext,
    scope: Scope,
    key: String,
    job: String,
    session: String,
    model: String,
    task: String,
    generation: i64,
    mut cancel: watch::Receiver<bool>,
) {
    let _permit = match acquire_permit(&supervisor, cancel.clone()).await {
        Ok(p) => p,
        Err(_) => return,
    };
    let estimate = mh_experiments::Usage {
        calls: 1,
        input: 300_000,
        output: 4096,
    };
    if store
        .host_reserve(
            &access, &scope, &job, "trial", "baseline", generation, 0, &estimate,
        )
        .is_err()
    {
        let _ = store.host_finish(&access, &scope, &job, generation, "budgetExceeded");
        return;
    }
    let scope_key = match serde_json::to_string(&scope) {
        Ok(v) => v,
        Err(_) => return,
    };
    let pending = json!({"pending":true,"stage":"baseline"}).to_string();
    if store
        .save_checkpoint(&access.actor_id, &scope_key, &job, generation, 0, &pending)
        .is_err()
    {
        let _ = store.host_finish(&access, &scope, &job, generation, "failed");
        return;
    }
    let run = "baseline".to_string();
    let request = mh_agent_loop::run_supervisor::StartRunRequest {
        session_id: session.clone(),
        request_id: run.clone(),
        parent_id: None,
        message: mh_agent_loop::run_supervisor::RunMessage {
            id: "request".into(),
            text: task,
        },
        model,
        options: json!({"toolAllowlist":[]}),
    };
    {
        let mut jobs = supervisor.jobs.lock().await;
        let Some(running) = jobs.get_mut(&key) else {
            return;
        };
        if *cancel.borrow() {
            return;
        }
        match service.start_run(request).await {
            Ok(_) => running.children.push((session.clone(), run.clone())),
            Err(_) => {
                let _ = store.host_settle(&access, &scope, &job, "baseline", generation, None);
                let _ = store.host_finish(&access, &scope, &job, generation, "failed");
                return;
            }
        }
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(3600));
    tokio::pin!(deadline);
    let mut stopping = false;
    loop {
        let snapshot = match service.get_run(&session, &run) {
            Ok(s) => s,
            Err(_) => {
                let _ = store.host_settle(&access, &scope, &job, "baseline", generation, None);
                let _ = store.host_finish(&access, &scope, &job, generation, "failed");
                return;
            }
        };
        if matches!(
            snapshot.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            let _ = store.host_settle(&access, &scope, &job, "baseline", generation, None);
            if snapshot.status == "completed" && !stopping && !*cancel.borrow() {
                let bytes = match serde_json::to_vec(&snapshot) {
                    Ok(b) => b,
                    Err(_) => return,
                };
                match store.put_artifact(&access.actor_id, &scope_key, &job, &bytes) {
                    Ok(digest) => {
                        let checkpoint =
                            json!({"pending":false,"stage":"baseline","artifact":digest})
                                .to_string();
                        if store
                            .save_checkpoint(
                                &access.actor_id,
                                &scope_key,
                                &job,
                                generation,
                                1,
                                &checkpoint,
                            )
                            .is_ok()
                        {
                            let _ =
                                store.host_finish(&access, &scope, &job, generation, "completed");
                        } else {
                            let _ = store.host_finish(&access, &scope, &job, generation, "failed");
                        }
                    }
                    Err(_) => {
                        let _ = store.host_finish(&access, &scope, &job, generation, "failed");
                    }
                }
            } else if !*cancel.borrow() {
                let _ = store.host_finish(
                    &access,
                    &scope,
                    &job,
                    generation,
                    if stopping { "budgetExceeded" } else { "failed" },
                );
            }
            return;
        }
        tokio::select! {
            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{},
            _=cancel.changed(), if !stopping => {stopping=true;let _=service.cancel_run(&session,&run);},
            _=&mut deadline, if !stopping => {stopping=true;let _=service.cancel_run(&session,&run);},
        }
    }
}
impl Learning {
    /// Caller must reserve budget and persist the pending stage before dispatch.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn model_text(
        &self,
        access: &AccessContext,
        scope: &Scope,
        job: &str,
        model: &str,
        prompt: &str,
        frozen_context: &str,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<String, String> {
        access.authorize(scope).map_err(|e| e.to_string())?;
        if !model.starts_with("api:") {
            return Err("UNSUPPORTED_ENGINE".into());
        }
        if prompt.is_empty() || prompt.len() > 65536 {
            return Err("INVALID_INPUT: prompt".into());
        }
        let manager = self.manager.upgrade().ok_or("plugin manager unavailable")?;
        let service = manager
            .context()
            .try_service::<AgentLoopService>()
            .map_err(|e| e.to_string())?;
        let child = format!("{job}-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed));
        self.sessions.create_internal(&child, "improvement", job)?;
        self.context.freeze_context(&child, frozen_context)?;
        let run = "stage".to_string();
        let job_key = key(access, scope, job)?;
        let request = mh_agent_loop::run_supervisor::StartRunRequest {
            session_id: child.clone(),
            request_id: run.clone(),
            parent_id: None,
            message: mh_agent_loop::run_supervisor::RunMessage {
                id: "request".into(),
                text: prompt.into(),
            },
            model: model.into(),
            options: json!({"toolAllowlist":[]}),
        };
        {
            let mut jobs = self.execution.jobs.lock().await;
            let running = jobs.get_mut(&job_key).ok_or("job is not supervised")?;
            if *cancel.borrow() || *running.cancel.borrow() {
                return Err("cancelled".into());
            }
            service.start_run(request).await?;
            running.children.push((child.clone(), run.clone()));
        }
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(deadline);
        let mut stopping = false;
        loop {
            let snapshot = service.get_run(&child, &run)?;
            if snapshot.status == "completed" && !stopping && !*cancel.borrow() {
                let output = snapshot
                    .parts
                    .iter()
                    .filter(|part| part.kind == "text")
                    .filter_map(|part| part.data["text"].as_str())
                    .collect::<String>();
                return if output.is_empty() {
                    Err("empty model output".into())
                } else {
                    Ok(output)
                };
            }
            if matches!(
                snapshot.status.as_str(),
                "completed" | "failed" | "cancelled"
            ) {
                return Err(snapshot
                    .error
                    .unwrap_or_else(|| "cancelled or failed".into()));
            }
            tokio::select! {
                _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{},
                _=cancel.changed(),if !stopping => {stopping=true;let _=service.cancel_run(&child,&run);},
                _=&mut deadline,if !stopping => {stopping=true;let _=service.cancel_run(&child,&run);},
            }
        }
    }
}
#[cfg(test)]
pub(crate) mod test_credentials {
    use std::sync::Arc;
    pub struct Cleanup {
        providers: Arc<crate::providers::ProviderService>,
        ids: Vec<String>,
    }
    impl Cleanup {
        pub fn new(providers: Arc<crate::providers::ProviderService>, ids: Vec<String>) -> Self {
            Self { providers, ids }
        }
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let providers = self.providers.clone();
            let ids = std::mem::take(&mut self.ids);
            let result = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("cleanup runtime");
                rt.block_on(async {
                    for id in ids {
                        providers.delete(&id).await.expect("delete test provider");
                    }
                });
            })
            .join();
            if result.is_err() && !std::thread::panicking() {
                panic!("test provider cleanup failed");
            }
        }
    }
    pub fn id() -> String {
        use sha2::Digest;
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let hash = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{:?}{:?}", a.path(), b.path()).as_bytes())
        );
        format!("test-{}", &hash[..32])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[tokio::test]
    async fn baseline_uses_native_provider_without_tools_and_persists_output() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::start(crate::RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap();
        runtime.sessions.create("parent").unwrap();
        let provider_id = test_credentials::id();
        let _cleanup = test_credentials::Cleanup::new(
            runtime.plugins.providers().unwrap(),
            vec![provider_id.clone()],
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..i]);
                    let len = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= i + 4 + len {
                        let body: Value =
                            serde_json::from_slice(&request[i + 4..i + 4 + len]).unwrap();
                        sent.send(body).unwrap();
                        break;
                    }
                }
            }
            let body=concat!("data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"baseline output\"},\"finish_reason\":null}]}\n\n","data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n","data: [DONE]\n\n");
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        runtime
            .plugins
            .providers()
            .unwrap()
            .save(crate::providers::ProviderInput {
                id: provider_id.clone(),
                name: "mock".into(),
                base_url: format!("http://{address}/v1"),
                models: vec!["model".into()],
                api_key: Some("test".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let host = runtime.learning.access("parent").unwrap();
        let scope = Scope::Session {
            session_id: "parent".into(),
        };
        let job = runtime
            .learning
            .execution_call(
                "experiments",
                "create",
                &host,
                &scope,
                json!({"task":"answer briefly","model":format!("api:{provider_id}/model"),"budget":4,"k":1}),
            )
            .await
            .unwrap();
        let id = job["id"].as_str().unwrap();
        runtime
            .learning
            .execution_call("experiments", "start", &host, &scope, json!({"id":id}))
            .await
            .unwrap();
        let body = tokio::time::timeout(std::time::Duration::from_secs(20), received)
            .await
            .unwrap()
            .unwrap();
        assert!(body
            .get("tools")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)));
        assert_eq!(body["max_tokens"], 4096);
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let view = runtime
                    .learning
                    .experiments
                    .host_get(&host, &scope, id)
                    .unwrap();
                if view.state != "running" {
                    break view;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.state, "completed");
        assert!(result.usage[0].unknown);
        let comparison = runtime
            .learning
            .execution_call("experiments", "compare", &host, &scope, json!({"id":id}))
            .await
            .unwrap();
        assert!(comparison["artifact"].as_str().is_some());
        server.join().unwrap();
    }
}
/// ComponentDiff.diff is a complete UTF-8 replacement, not a unified diff.
fn materialize_bundle(
    base: &BTreeMap<String, String>,
    diffs: &[mh_improvement::ComponentDiff],
) -> Result<(BTreeMap<String, String>, String), String> {
    let mut next = base.clone();
    let mut seen = std::collections::BTreeSet::new();
    for diff in diffs {
        let path = match diff.component.as_str() {
            "systemPrompt" => "system-prompt.txt",
            "toolGuidance" => "tool-guidance.txt",
            "memory" => "memory.txt",
            "skills" => "skills.txt",
            _ => return Err("UNSUPPORTED_COMPONENT".into()),
        };
        if diff.path != path || diff.diff.len() > 65536 || !seen.insert(diff.component.clone()) {
            return Err("INVALID_COMPONENT".into());
        }
        next.insert(diff.component.clone(), diff.diff.clone());
    }
    let encoded = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
    if encoded.len() > 262144 {
        return Err("bundle exceeds limit".into());
    }
    let digest = format!("{:x}", Sha256::digest(&encoded));
    Ok((next, digest))
}
struct PromptExecutor {
    host: Arc<Learning>,
    access: AccessContext,
    scope: Scope,
    job: String,
    generation: i64,
    task: String,
    model: String,
    frozen: String,
    cancel: watch::Receiver<bool>,
    handle: tokio::runtime::Handle,
    bundles: BTreeMap<String, BTreeMap<String, String>>,
    current: String,
    initial: String,
    mode: mh_improvement::Mode,
    rounds: BTreeMap<u8, String>,
    final_bundles: Vec<String>,
    library: Vec<Value>,
    count: u32,
}
impl mh_improvement::StageExecutor for PromptExecutor {
    fn execute(
        &mut self,
        step: &mh_improvement::Step,
        evidence: &[mh_improvement::Record],
    ) -> Result<mh_improvement::Outcome, String> {
        use mh_improvement::{Outcome, Stage};
        if *self.cancel.borrow() {
            return Err("cancelled".into());
        }
        let base = match step.stage {
            Stage::Baseline => self.initial.clone(),
            Stage::Reflect => self.current.clone(),
            Stage::Final => self
                .rounds
                .get(&step.round)
                .ok_or("missing composed bundle")?
                .clone(),
            _ if self.mode == mh_improvement::Mode::Parallel => self.initial.clone(),
            _ => self.current.clone(),
        };
        let mut bundle = self.bundles.get(&base).ok_or("missing bundle")?.clone();
        if matches!(step.stage, Stage::GeneralProbe | Stage::DomainProbe) {
            let target = if step.stage == Stage::GeneralProbe {
                mh_improvement::Scope::General
            } else {
                mh_improvement::Scope::Domain
            };
            let proposal = evidence
                .iter()
                .rev()
                .find_map(|r| match &r.outcome {
                    Outcome::Proposal { proposal }
                        if proposal.scope == target && r.step.round == step.round =>
                    {
                        Some(proposal)
                    }
                    _ => None,
                })
                .ok_or("missing proposal")?;
            bundle = materialize_bundle(&bundle, &proposal.component_diffs)?.0;
        }
        let mut context = self.frozen.clone();
        for (name, value) in &bundle {
            context.push_str(&format!("\nTask-local {name}:\n{value}\n"));
        }
        let shape=match step.stage {
            Stage::GeneralProposal|Stage::DomainProposal => "Return only JSON: {\"kind\":\"proposal\",\"proposal\":{\"targetDeficiency\":\"...\",\"scope\":\"general or domain as requested\",\"baseDigest\":\"BASE_DIGEST\",\"componentDiffs\":[{\"component\":\"systemPrompt\",\"path\":\"system-prompt.txt\",\"diff\":\"complete replacement text\"}],\"hypothesis\":\"...\",\"evidenceRefs\":[\"digest from prior evidence\"]}}. Allowed components and fixed paths: systemPrompt/system-prompt.txt, toolGuidance/tool-guidance.txt, memory/memory.txt, skills/skills.txt. The diff is full replacement text.",
            Stage::Compose => "Return only JSON: {\"kind\":\"compose\",\"compose\":{\"adoptedDiffs\":[],\"rejectedDiffs\":[],\"reasons\":\"...\",\"evidenceRefs\":[],\"resultDigest\":\"host-will-compute\"}}. Choose only diffs in proposals; an empty adoptedDiffs keeps baseline.",
            Stage::Reflect => "Return only JSON: {\"kind\":\"noEntry\"} or {\"kind\":\"reflection\",\"reflection\":{\"cue\":\"...\",\"lesson\":\"...\",\"evidenceRefs\":[],\"limitations\":\"...\",\"bundleDigest\":\"...\"}}.",
            _ => "Solve the task with the supplied task-local instructions. Return the result as plain text.",
        };
        let candidates = if matches!(step.stage, Stage::GeneralProposal | Stage::DomainProposal) {
            serde_json::to_string(&self.library).map_err(|e| e.to_string())?
        } else {
            "[]".into()
        };
        let required_scope = match step.stage {
            Stage::GeneralProposal => "general",
            Stage::DomainProposal => "domain",
            _ => "not applicable",
        };
        let prompt = format!(
            "Task: {}\nStage: {:?}\nRequired proposal scope: {}\nBase digest: {}\nPrior evidence: {}\nRelevant experience candidates, provisional entries require independent checking: {}\n{}",
            self.task,
            step.stage,
            required_scope,
            base,
            serde_json::to_string(evidence).map_err(|e| e.to_string())?,
            candidates,
            shape.replace("BASE_DIGEST", &base)
        );
        let _permit = self
            .handle
            .block_on(acquire_permit(&self.host.execution, self.cancel.clone()))?;
        let reservation = format!("stage-{}", self.count);
        self.count += 1;
        self.host
            .experiments
            .host_reserve(
                &self.access,
                &self.scope,
                &self.job,
                "trial",
                &reservation,
                self.generation,
                0,
                &mh_experiments::Usage {
                    calls: 1,
                    input: 100_000,
                    output: 4096,
                },
            )
            .map_err(error)?;
        let result = self.handle.block_on(self.host.model_text(
            &self.access,
            &self.scope,
            &self.job,
            &self.model,
            &prompt,
            &context,
            self.cancel.clone(),
        ));
        self.host
            .experiments
            .host_settle(
                &self.access,
                &self.scope,
                &self.job,
                &reservation,
                self.generation,
                None,
            )
            .map_err(error)?;
        let output = result?;
        match step.stage {
            Stage::Baseline | Stage::GeneralProbe | Stage::DomainProbe | Stage::Final => {
                let scope_key = serde_json::to_string(&self.scope).map_err(|e| e.to_string())?;
                let digest = self
                    .host
                    .experiments
                    .put_artifact(
                        &self.access.actor_id,
                        &scope_key,
                        &self.job,
                        output.as_bytes(),
                    )
                    .map_err(error)?;
                if step.stage == Stage::Final {
                    self.final_bundles.push(base.clone());
                }
                Ok(Outcome::Evidence {
                    digest,
                    content: output,
                })
            }
            Stage::Compose => {
                let mut outcome: Outcome = serde_json::from_str(&output)
                    .map_err(|e| format!("invalid model JSON: {e}"))?;
                if let Outcome::Compose { compose } = &mut outcome {
                    let (next, digest) = materialize_bundle(&bundle, &compose.adopted_diffs)?;
                    let key = serde_json::to_string(&self.scope).map_err(|e| e.to_string())?;
                    let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
                    let stored = self
                        .host
                        .experiments
                        .put_artifact(&self.access.actor_id, &key, &self.job, &bytes)
                        .map_err(error)?;
                    if stored != digest {
                        return Err("bundle digest mismatch".into());
                    }
                    let permitted: Vec<_> = evidence
                        .iter()
                        .filter(|r| r.step.round == step.round)
                        .filter_map(|r| match &r.outcome {
                            Outcome::Proposal { proposal } => Some(&proposal.component_diffs),
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    if compose
                        .adopted_diffs
                        .iter()
                        .chain(&compose.rejected_diffs)
                        .any(|d| !permitted.contains(&d))
                    {
                        return Err("compose diff is not from this round".into());
                    }
                    self.bundles.insert(digest.clone(), next);
                    self.rounds.insert(step.round, digest.clone());
                    if self.mode == mh_improvement::Mode::Sequential {
                        self.current = digest.clone();
                    }
                    compose.result_digest = digest;
                }
                Ok(outcome)
            }
            _ => serde_json::from_str(&output).map_err(|e| format!("invalid model JSON: {e}")),
        }
    }
    fn select(&mut self, candidates: &[String]) -> Result<usize, String> {
        let _permit = self
            .handle
            .block_on(acquire_permit(&self.host.execution, self.cancel.clone()))?;
        let reservation = format!("stage-{}", self.count);
        self.count += 1;
        self.host
            .experiments
            .host_reserve(
                &self.access,
                &self.scope,
                &self.job,
                "trial",
                &reservation,
                self.generation,
                0,
                &mh_experiments::Usage {
                    calls: 1,
                    input: 100_000,
                    output: 4096,
                },
            )
            .map_err(error)?;
        let prompt=format!("Select the best response to the task. Return only JSON {{\"index\":0}}, using a zero-based index. Task: {}\nResponses: {}",self.task,serde_json::to_string(candidates).map_err(|e|e.to_string())?);
        let result = self.handle.block_on(self.host.model_text(
            &self.access,
            &self.scope,
            &self.job,
            &self.model,
            &prompt,
            &self.frozen,
            self.cancel.clone(),
        ));
        self.host
            .experiments
            .host_settle(
                &self.access,
                &self.scope,
                &self.job,
                &reservation,
                self.generation,
                None,
            )
            .map_err(error)?;
        let value: Value = serde_json::from_str(&result?).map_err(|e| e.to_string())?;
        let index = value
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|i| *i < candidates.len())
            .ok_or("invalid selector index")?;
        self.current = self
            .final_bundles
            .get(index)
            .ok_or("missing selected bundle")?
            .clone();
        Ok(index)
    }
}
#[allow(clippy::too_many_arguments)]
fn run_improvement(
    host: Arc<Learning>,
    access: AccessContext,
    scope: Scope,
    job: String,
    generation: i64,
    task: String,
    model: String,
    frozen: String,
    base_digest: String,
    library_revision: String,
    library: Vec<Value>,
    budget: i64,
    k: u8,
    mode: mh_improvement::Mode,
    resume: Option<(i64, String)>,
    cancel: watch::Receiver<bool>,
    handle: tokio::runtime::Handle,
) {
    let request = mh_improvement::Request {
        mode: mode.clone(),
        k,
        budget: budget as u32,
        input_digest: base_digest.clone(),
        library_revision: library_revision.clone(),
        allowed_components: vec![
            "systemPrompt".into(),
            "toolGuidance".into(),
            "memory".into(),
            "skills".into(),
        ],
    };
    let (mut seq, mut journal) = match resume {
        Some((seq, data)) => match serde_json::from_str::<mh_improvement::Journal>(&data) {
            Ok(mut journal) => match journal.resume(&base_digest, &library_revision) {
                Ok(()) => (seq, journal),
                Err(_) => {
                    let _ = host
                        .experiments
                        .host_finish(&access, &scope, &job, generation, "failed");
                    return;
                }
            },
            Err(_) => {
                let _ = host
                    .experiments
                    .host_finish(&access, &scope, &job, generation, "failed");
                return;
            }
        },
        None => match mh_improvement::Journal::new(request) {
            Ok(journal) => (0, journal),
            Err(_) => {
                let _ = host
                    .experiments
                    .host_finish(&access, &scope, &job, generation, "failed");
                return;
            }
        },
    };
    let mut bundles = BTreeMap::new();
    bundles.insert(base_digest.clone(), BTreeMap::new());
    let mut executor = PromptExecutor {
        host: host.clone(),
        access: access.clone(),
        scope: scope.clone(),
        job: job.clone(),
        generation,
        task,
        model,
        frozen,
        cancel: cancel.clone(),
        handle,
        bundles,
        current: base_digest.clone(),
        initial: base_digest,
        mode,
        rounds: BTreeMap::new(),
        final_bundles: Vec::new(),
        library,
        count: 0,
    };
    let key = match serde_json::to_string(&scope) {
        Ok(v) => v,
        Err(_) => return,
    };
    if seq > 0 && restore_executor(&mut executor, journal.records()).is_err() {
        let _ = host
            .experiments
            .host_finish(&access, &scope, &job, generation, "failed");
        return;
    }
    let result = journal.run_persisted(&mut executor, |j| {
        let data = serde_json::to_string(j).map_err(|e| e.to_string())?;
        seq = host
            .experiments
            .save_checkpoint(&access.actor_id, &key, &job, generation, seq, &data)
            .map_err(error)?;
        Ok(())
    });
    if !*cancel.borrow() {
        let completed = result.is_ok();
        if host
            .experiments
            .host_finish(
                &access,
                &scope,
                &job,
                generation,
                if completed { "completed" } else { "failed" },
            )
            .is_ok()
            && completed
        {
            if let Err(e) = publish_reflection(&host, &access, &scope, &job, &journal, &executor) {
                eprintln!("experience publication skipped: {e}");
            }
        }
    }
}
#[cfg(test)]
mod bundle_tests {
    use super::*;
    #[test]
    fn replacements_are_scoped_and_digest_is_stable() {
        let base = BTreeMap::new();
        let diff = mh_improvement::ComponentDiff {
            component: "systemPrompt".into(),
            path: "system-prompt.txt".into(),
            diff: "new instructions".into(),
        };
        let (next, digest) = materialize_bundle(&base, std::slice::from_ref(&diff)).unwrap();
        assert_eq!(next["systemPrompt"], "new instructions");
        assert_eq!(
            digest,
            materialize_bundle(&base, std::slice::from_ref(&diff))
                .unwrap()
                .1
        );
        let mut unsafe_diff = diff;
        unsafe_diff.path = "../secret".into();
        assert!(materialize_bundle(&base, &[unsafe_diff]).is_err());
        let native = mh_improvement::ComponentDiff {
            component: "scripts".into(),
            path: "script".into(),
            diff: "unsafe".into(),
        };
        assert_eq!(
            materialize_bundle(&base, &[native]).unwrap_err(),
            "UNSUPPORTED_COMPONENT"
        );
    }
}
#[cfg(test)]
mod improvement_http_test {
    use super::*;
    use std::io::{Read, Write};
    async fn run_case(k: u64, mode: &str, publish: bool, resume: bool) {
        let dir = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::start(crate::RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap();
        runtime.sessions.create("parent").unwrap();
        let provider_id = test_credentials::id();
        let _cleanup = test_credentials::Cleanup::new(
            runtime.plugins.providers().unwrap(),
            vec![provider_id.clone()],
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..if resume {
                15
            } else if k == 1 {
                8
            } else {
                16
            } {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(15)))
                    .unwrap();
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(i) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..i]);
                        let length = header
                            .lines()
                            .find_map(|v| {
                                v.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= i + 4 + length {
                            break;
                        }
                    }
                }
                let body = String::from_utf8_lossy(&data);
                if body.contains("Stage: GeneralProbe") || body.contains("Stage: DomainProbe") {
                    assert!(body.contains("Task-local systemPrompt"));
                }
                if body.contains("Stage: Final") {
                    assert!(!body.contains("Task-local systemPrompt"));
                }
                let base = format!("{:x}", Sha256::digest(b"{}"));
                let content = if body.contains("Select the best response to the task") {
                    json!({"index":0}).to_string()
                } else if body.contains("Stage: GeneralProposal")
                    || body.contains("Stage: DomainProposal")
                {
                    let scope = if body.contains("Stage: GeneralProposal") {
                        "general"
                    } else {
                        "domain"
                    };
                    json!({"kind":"proposal","proposal":{"targetDeficiency":"more detail","scope":scope,"baseDigest":base,"componentDiffs":[{"component":"systemPrompt","path":"system-prompt.txt","diff":"Be precise"}],"hypothesis":"more precision","evidenceRefs":[]}}).to_string()
                } else if body.contains("Stage: Compose") {
                    json!({"kind":"compose","compose":{"adoptedDiffs":[],"rejectedDiffs":[],"reasons":"retain baseline","evidenceRefs":[],"resultDigest":"placeholder"}}).to_string()
                } else if body.contains("Stage: Reflect") {
                    if publish {
                        let evidence = format!("{:x}", Sha256::digest(b"verified result"));
                        json!({"kind":"reflection","reflection":{"cue":"answer","lesson":"answer with precision","evidenceRefs":[evidence],"limitations":"model evidence only","bundleDigest":base}}).to_string()
                    } else {
                        json!({"kind":"noEntry"}).to_string()
                    }
                } else {
                    "verified result".into()
                };
                let first=json!({"id":"x","object":"chat.completion.chunk","created":1,"model":"model","choices":[{"index":0,"delta":{"role":"assistant","content":content},"finish_reason":null}]}).to_string();
                let last=json!({"id":"x","object":"chat.completion.chunk","created":1,"model":"model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}).to_string();
                let response = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            }
        });
        runtime
            .plugins
            .providers()
            .unwrap()
            .save(crate::providers::ProviderInput {
                id: provider_id.clone(),
                name: "mock".into(),
                base_url: format!("http://{address}/v1"),
                models: vec!["model".into()],
                api_key: Some("test".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let host = runtime.learning.access("parent").unwrap();
        let scope = Scope::Session {
            session_id: "parent".into(),
        };
        let job = runtime
            .learning
            .execution_call(
                "improvement",
                "start",
                &host,
                &scope,
                json!({"task":"answer","model":format!("api:{provider_id}/model"),"budget":if k==1 {8} else {16},"k":k,"mode":mode}),
            )
            .await
            .unwrap();
        let id = job["id"].as_str().unwrap();
        let view = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let v = runtime
                    .learning
                    .experiments
                    .host_get(&host, &scope, id)
                    .unwrap();
                if v.state != "running" {
                    break v;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(view.state, "completed");
        let expected = if k == 1 { 8 } else { 16 };
        assert_eq!(view.usage.len(), expected);
        assert_eq!(view.stages.len(), expected * 2);
        if resume {
            let db =
                rusqlite::Connection::open(dir.path().join("data/learning/experiments.sqlite3"))
                    .unwrap();
            let scope_key = serde_json::to_string(&scope).unwrap();
            db.execute(
                "DELETE FROM checkpoints WHERE actor=?1 AND scope=?2 AND job=?3 AND seq>2",
                rusqlite::params![host.actor_id, scope_key, id],
            )
            .unwrap();
            db.execute("DELETE FROM reservations WHERE actor=?1 AND scope=?2 AND job=?3 AND id <> 'stage-0'",rusqlite::params![host.actor_id,scope_key,id]).unwrap();
            db.execute(
                "DELETE FROM estimates WHERE actor=?1 AND scope=?2 AND job=?3 AND id <> 'stage-0'",
                rusqlite::params![host.actor_id, scope_key, id],
            )
            .unwrap();
            db.execute("UPDATE jobs SET state='interrupted',generation=generation+1 WHERE actor=?1 AND scope=?2 AND id=?3",rusqlite::params![host.actor_id,scope_key,id]).unwrap();
            let original:String=db.query_row("SELECT journal FROM checkpoints WHERE actor=?1 AND scope=?2 AND job=?3 AND seq=2",rusqlite::params![host.actor_id,scope_key,id],|r|r.get(0)).unwrap();
            let mut uncertain: Value = serde_json::from_str(&original).unwrap();
            uncertain["pending"] = Value::Bool(true);
            db.execute("UPDATE checkpoints SET journal=?4 WHERE actor=?1 AND scope=?2 AND job=?3 AND seq=2",rusqlite::params![host.actor_id,scope_key,id,uncertain.to_string()]).unwrap();
            drop(db);
            let denied = runtime
                .learning
                .execution_call("improvement", "resume", &host, &scope, json!({"id":id}))
                .await
                .unwrap_err();
            assert!(denied.contains("UNCERTAIN_STAGE"));
            let db =
                rusqlite::Connection::open(dir.path().join("data/learning/experiments.sqlite3"))
                    .unwrap();
            db.execute("UPDATE checkpoints SET journal=?4 WHERE actor=?1 AND scope=?2 AND job=?3 AND seq=2",rusqlite::params![host.actor_id,scope_key,id,original]).unwrap();
            drop(db);
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    match runtime
                        .learning
                        .execution_call("improvement", "resume", &host, &scope, json!({"id":id}))
                        .await
                    {
                        Ok(_) => break,
                        Err(e) if e == "CONFLICT" => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await
                        }
                        Err(e) => panic!("resume failed: {e}"),
                    }
                }
            })
            .await
            .unwrap();
            let resumed = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let v = runtime
                        .learning
                        .experiments
                        .host_get(&host, &scope, id)
                        .unwrap();
                    if v.state != "running" {
                        break v;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(resumed.state, "completed");
            assert_eq!(resumed.usage.len(), 8);
            assert_eq!(resumed.stages.len(), 16);
        }
        server.join().unwrap();
        if publish {
            let entries = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let rows = runtime
                        .learning
                        .experience
                        .search(&host, &scope, "", 50)
                        .unwrap();
                    if !rows.is_empty() {
                        break rows;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                entries[0].status,
                minerva_plugin_sdk::experience::ExperienceStatus::Provisional
            );
            let next=runtime.learning.execution_call("improvement","start",&host,&scope,json!({"task":"answer another question","model":format!("api:{provider_id}/model"),"budget":8,"k":1})).await.unwrap();
            let next_id = next["id"].as_str().unwrap();
            let key = serde_json::to_string(&scope).unwrap();
            let spec = runtime
                .learning
                .experiments
                .artifact(
                    &host.actor_id,
                    &key,
                    next_id,
                    next["inputDigest"].as_str().unwrap(),
                )
                .unwrap();
            let spec: Value = serde_json::from_slice(&spec).unwrap();
            assert_eq!(spec["library"][0]["status"], "provisional");
            assert!(!spec["frozenContext"]
                .as_str()
                .unwrap()
                .contains("answer with precision"));
            let _ = runtime
                .learning
                .execution_call(
                    "improvement",
                    "cancel",
                    &host,
                    &scope,
                    json!({"id":next_id}),
                )
                .await;
        } else {
            assert!(runtime
                .learning
                .experience
                .search(&host, &scope, "", 50)
                .unwrap()
                .is_empty());
        }
    }
    #[tokio::test]
    async fn k1_stages_persist() {
        run_case(1, "sequential", true, false).await;
    }
    #[tokio::test]
    async fn k2_sequential_stages_persist() {
        run_case(2, "sequential", false, false).await;
    }
    #[tokio::test]
    async fn resumes_after_completed_stage_without_replaying_it() {
        run_case(1, "sequential", false, true).await;
    }
    #[tokio::test]
    async fn k2_parallel_stages_persist() {
        run_case(2, "parallel", false, false).await;
    }
}
fn publish_reflection(
    host: &Learning,
    access: &AccessContext,
    scope: &Scope,
    job: &str,
    journal: &mh_improvement::Journal,
    executor: &PromptExecutor,
) -> Result<(), String> {
    use mh_improvement::Outcome;
    use minerva_plugin_sdk::{
        experience::{BundleComponent, BundleManifest, ComponentKind, PublishExperience},
        learning::ArtifactRef,
    };
    let reflection = journal
        .records()
        .iter()
        .rev()
        .find_map(|r| match &r.outcome {
            Outcome::Reflection { reflection } => Some(reflection),
            _ => None,
        });
    let Some(reflection) = reflection else {
        return Ok(());
    };
    if reflection.evidence_refs.is_empty() || reflection.bundle_digest != executor.current {
        return Err("reflection lacks evidence or refers to a different bundle".into());
    }
    let selected = executor
        .bundles
        .get(&executor.current)
        .ok_or("selected bundle missing")?;
    let mut files = BTreeMap::new();
    let mut components = Vec::new();
    for (name, value) in selected {
        let (kind, path) = match name.as_str() {
            "systemPrompt" => (ComponentKind::SystemPrompt, "system-prompt.txt"),
            "toolGuidance" => (ComponentKind::ToolGuidance, "tool-guidance.txt"),
            "memory" => (ComponentKind::Memory, "memory.txt"),
            "skills" => (ComponentKind::Skills, "skills.txt"),
            _ => return Err("unsupported component".into()),
        };
        let bytes = value.as_bytes().to_vec();
        components.push(BundleComponent {
            kind,
            path: path.into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes: bytes.len() as u64,
            adapter_version: "prompt-replacement-v1".into(),
        });
        files.insert(path.to_string(), bytes);
    }
    let mut manifest = BundleManifest {
        schema_version: 1,
        digest: String::new(),
        parent_digest: None,
        components,
        scope: scope.clone(),
        source_refs: Vec::new(),
    };
    manifest.digest =
        mh_experience::ExperienceService::bundle_digest(&manifest).map_err(|e| e.to_string())?;
    host.experience
        .store_bundle(access, &manifest, &files)
        .map_err(|e| e.to_string())?;
    let key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
    let mut refs = Vec::new();
    for digest in &reflection.evidence_refs {
        let data = host
            .experiments
            .artifact(&access.actor_id, &key, job, digest)
            .map_err(error)?;
        refs.push(ArtifactRef {
            artifact_id: digest.clone(),
            digest: digest.clone(),
            media_type: "text/plain".into(),
            bytes: data.len() as u64,
        });
    }
    let reason = journal
        .records()
        .iter()
        .rev()
        .find_map(|r| match &r.outcome {
            Outcome::Compose { compose } => Some(compose.reasons.clone()),
            _ => None,
        })
        .ok_or("missing composition")?;
    host.experience
        .publish(
            access,
            PublishExperience {
                id: format!("experience-{job}"),
                experiment_id: job.into(),
                domain: "task".into(),
                scope: scope.clone(),
                cue: reflection.cue.clone(),
                lesson: reflection.lesson.clone(),
                evidence_refs: refs,
                bundle_digest: manifest.digest,
                adoption_reason: reason,
                limitations: reflection.limitations.clone(),
                confidence: "provisional".into(),
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod cancellation_http_test {
    use super::*;
    use std::io::Read;
    #[tokio::test]
    async fn cancel_joins_inflight_native_child() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::start(crate::RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap();
        runtime.sessions.create("parent").unwrap();
        let provider_id = test_credentials::id();
        let _cleanup = test_credentials::Cleanup::new(
            runtime.plugins.providers().unwrap(),
            vec![provider_id.clone()],
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (seen, rx) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut data = [0u8; 2048];
            let _ = socket.read(&mut data).unwrap();
            seen.send(()).unwrap();
            let _ = wait.recv();
        });
        runtime
            .plugins
            .providers()
            .unwrap()
            .save(crate::providers::ProviderInput {
                id: provider_id.clone(),
                name: "mock".into(),
                base_url: format!("http://{addr}/v1"),
                models: vec!["model".into()],
                api_key: Some("test".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let host = runtime.learning.access("parent").unwrap();
        let scope = Scope::Session {
            session_id: "parent".into(),
        };
        let job = runtime
            .learning
            .execution_call(
                "experiments",
                "create",
                &host,
                &scope,
                json!({"task":"wait","model":format!("api:{provider_id}/model"),"budget":1,"k":1}),
            )
            .await
            .unwrap();
        let id = job["id"].as_str().unwrap();
        runtime
            .learning
            .execution_call("experiments", "start", &host, &scope, json!({"id":id}))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            runtime.learning.execution_call(
                "experiments",
                "cancel",
                &host,
                &scope,
                json!({"id":id}),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let status = runtime
            .learning
            .experiments
            .host_get(&host, &scope, id)
            .unwrap();
        assert_eq!(status.state, "cancelled");
        assert!(status.usage[0].unknown);
        release.send(()).unwrap();
        server.join().unwrap();
    }
}
fn restore_executor(
    executor: &mut PromptExecutor,
    records: &[mh_improvement::Record],
) -> Result<(), String> {
    use mh_improvement::{Outcome, Stage};
    let scope = serde_json::to_string(&executor.scope).map_err(|e| e.to_string())?;
    for record in records {
        if let Outcome::Evidence { digest, content } = &record.outcome {
            let bytes = executor
                .host
                .experiments
                .artifact(&executor.access.actor_id, &scope, &executor.job, digest)
                .map_err(error)?;
            if bytes != content.as_bytes() {
                return Err("evidence changed".into());
            }
        }
        match &record.outcome {
            Outcome::Compose { compose } => {
                let base = if executor.mode == mh_improvement::Mode::Parallel {
                    &executor.initial
                } else {
                    &executor.current
                };
                let (bundle, digest) = materialize_bundle(
                    executor.bundles.get(base).ok_or("base bundle missing")?,
                    &compose.adopted_diffs,
                )?;
                if digest != compose.result_digest {
                    return Err("bundle changed".into());
                }
                executor.bundles.insert(digest.clone(), bundle);
                executor.rounds.insert(record.step.round, digest.clone());
                if executor.mode == mh_improvement::Mode::Sequential {
                    executor.current = digest;
                }
            }
            Outcome::Evidence { .. } if record.step.stage == Stage::Final => {
                executor.final_bundles.push(
                    executor
                        .rounds
                        .get(&record.step.round)
                        .ok_or("final bundle missing")?
                        .clone(),
                )
            }
            Outcome::Selected { index } => {
                executor.current = executor
                    .final_bundles
                    .get(*index)
                    .ok_or("selected bundle missing")?
                    .clone()
            }
            _ => {}
        }
    }
    executor.count = records.len() as u32;
    Ok(())
}

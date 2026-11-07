//! One bounded supervisor per configured server. Transport generations own their tools.
//! DSH connection/generation/deadline lifetimes mapped to Tokio tasks and cancellation.
use crate::{
    credential_store::{CredentialScope, CredentialStore},
    network_policy::EndpointPolicy,
    tools::{Cancellation, ToolDefinition, ToolRegistry},
};
pub use minerva_plugin_sdk::mcp::*;
use rmcp::{
    model::{CallToolRequestParams, CallToolResponse},
    service::RunningService,
    transport::{
        async_rw::AsyncRwTransport, streamable_http_client::StreamableHttpClientTransportConfig,
        StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{process::Command, task::JoinHandle};
const STARTUP: Duration = Duration::from_secs(20);
const CALL: Duration = Duration::from_secs(60);
const CLOSE: Duration = Duration::from_secs(3);
const FRAME: usize = 1024 * 1024;
struct Entry {
    config: McpServerConfig,
    state: Arc<Mutex<(String, usize)>>,
    stop: Cancellation,
    task: Option<JoinHandle<()>>,
}
pub struct McpManager {
    path: PathBuf,
    scope: CredentialScope,
    credentials: Arc<dyn CredentialStore>,
    registry: Arc<ToolRegistry>,
    policy: EndpointPolicy,
    entries: Mutex<BTreeMap<String, Entry>>,
    edits: tokio::sync::Mutex<()>,
    paused: AtomicBool,
}
impl cordis::Service for McpManager {
    const NAME: &'static str = "studio.mcp";
}
impl McpManager {
    pub fn new(
        path: PathBuf,
        scope: CredentialScope,
        credentials: Arc<dyn CredentialStore>,
        registry: Arc<ToolRegistry>,
        policy: EndpointPolicy,
    ) -> Result<Arc<Self>, String> {
        let configs: Vec<McpServerConfig> = if path.exists() {
            if std::fs::metadata(&path)
                .map_err(|_| "MCP settings unavailable")?
                .len()
                > FRAME as u64
            {
                return Err("MCP settings too large".into());
            }
            serde_json::from_slice(&std::fs::read(&path).map_err(|_| "MCP settings unavailable")?)
                .map_err(|_| "Invalid MCP settings")?
        } else {
            vec![]
        };
        if configs.len() > 64 {
            return Err("MCP server limit exceeded".into());
        }
        let manager = Arc::new(Self {
            path,
            scope,
            credentials,
            registry,
            policy,
            entries: Mutex::default(),
            edits: tokio::sync::Mutex::new(()),
            paused: AtomicBool::new(false),
        });
        for config in configs {
            manager.validate(&config)?;
            let id = config.id.clone();
            if manager
                .entries
                .lock()
                .unwrap()
                .insert(id, Self::entry(config))
                .is_some()
            {
                return Err("Duplicate MCP id".into());
            }
        }
        Ok(manager)
    }
    fn entry(config: McpServerConfig) -> Entry {
        Entry {
            config,
            state: Arc::new(Mutex::new(("disconnected".into(), 0))),
            stop: Cancellation::default(),
            task: None,
        }
    }
    fn credential_key(&self, id: &str) -> String {
        format!(
            "mcp:{:x}:{id}",
            Sha256::digest(self.scope.as_str().as_bytes())
        )
    }
    fn validate(&self, config: &McpServerConfig) -> Result<(), String> {
        if config.id.is_empty()
            || config.id.len() > 48
            || !config
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("Invalid MCP server id".into());
        }
        match &config.transport {
            McpTransport::Stdio { command, args } => {
                if self.policy == EndpointPolicy::Hosted {
                    return Err("Hosted MCP stdio is disabled".into());
                }
                if command.is_empty()
                    || command.len() > 8192
                    || command.contains('\0')
                    || args.len() > 128
                    || args.iter().any(|a| a.len() > 8192 || a.contains('\0'))
                {
                    return Err("Invalid MCP command".into());
                }
            }
            McpTransport::StreamableHttp { url } => self.policy.require_safe_transport(url)?,
        }
        Ok(())
    }
    fn get_credentials(&self, id: &str) -> Result<McpCredentials, String> {
        self.credentials
            .get(&self.scope, &self.credential_key(id))?
            .map(|s| serde_json::from_str(&s).map_err(|_| "Invalid MCP credentials".into()))
            .unwrap_or_else(|| Ok(McpCredentials::default()))
    }
    pub fn list(&self) -> Vec<McpServerView> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .map(|entry| {
                let (state, tool_count) = entry.state.lock().unwrap().clone();
                McpServerView {
                    config: entry.config.clone(),
                    state,
                    tool_count,
                    has_credentials: self
                        .credentials
                        .get(&self.scope, &self.credential_key(&entry.config.id))
                        .ok()
                        .flatten()
                        .is_some(),
                }
            })
            .collect()
    }
    fn persist(&self, configs: Vec<McpServerConfig>) -> Result<(), String> {
        if configs.len() > 64 {
            return Err("MCP server limit exceeded".into());
        }
        let bytes = serde_json::to_vec_pretty(&configs).map_err(|_| "Invalid MCP settings")?;
        if bytes.len() > FRAME {
            return Err("MCP settings too large".into());
        }
        let dir = self.path.parent().ok_or("Invalid MCP settings path")?;
        std::fs::create_dir_all(dir).map_err(|_| "MCP settings unavailable")?;
        let mut temp =
            tempfile::NamedTempFile::new_in(dir).map_err(|_| "MCP settings unavailable")?;
        temp.write_all(&bytes)
            .map_err(|_| "MCP settings unavailable")?;
        temp.as_file()
            .sync_all()
            .map_err(|_| "MCP settings unavailable")?;
        temp.persist(&self.path)
            .map_err(|_| "MCP settings unavailable")?;
        Ok(())
    }
    pub async fn start(self: &Arc<Self>) {
        let ids = self
            .entries
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.launch(&id);
        }
    }
    fn launch(self: &Arc<Self>, id: &str) {
        if self.paused.load(Ordering::SeqCst) {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(id) else {
            return;
        };
        if !entry.config.enabled || entry.task.is_some() {
            return;
        }
        let config = entry.config.clone();
        let stop = entry.stop.clone();
        let state = entry.state.clone();
        let manager = self.clone();
        entry.task = Some(tokio::spawn(async move {
            manager.supervise(config, stop, state).await;
        }));
    }
    async fn stop(&self, id: &str) {
        let task = {
            let mut entries = self.entries.lock().unwrap();
            entries.get_mut(id).and_then(|e| {
                e.stop.cancel();
                e.task.take()
            })
        };
        if let Some(mut task) = task {
            if tokio::time::timeout(CLOSE + Duration::from_secs(1), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
    pub async fn save(
        self: &Arc<Self>,
        input: McpServerInput,
    ) -> Result<Vec<McpServerView>, String> {
        let _edit = self.edits.lock().await;
        self.validate(&input.config)?;
        if let Some(secret) = &input.credentials {
            if secret.env.len() + secret.headers.len() > 128
                || secret
                    .env
                    .iter()
                    .chain(secret.headers.iter())
                    .any(|(k, v)| {
                        k.is_empty()
                            || k.len() > 256
                            || v.len() > 8192
                            || k.contains(['\0', '\r', '\n'])
                            || v.contains('\0')
                    })
            {
                return Err("Invalid MCP credentials".into());
            }
            for (name, value) in &secret.headers {
                reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| "Invalid MCP header")?;
                reqwest::header::HeaderValue::from_str(value).map_err(|_| "Invalid MCP header")?;
            }
        }
        let mut configs = self
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|e| e.config.clone())
            .filter(|c| c.id != input.config.id)
            .collect::<Vec<_>>();
        configs.push(input.config.clone());
        // Persist credentials outside configuration. Key names include scope even for legacy keyring adapters.
        let key = self.credential_key(&input.config.id);
        let previous = if input.credentials.is_some() {
            self.credentials.get(&self.scope, &key)?
        } else {
            None
        };
        let changed = input.credentials.is_some();
        if let Some(secret) = input.credentials {
            if secret.env.is_empty() && secret.headers.is_empty() {
                self.credentials.delete(&self.scope, &key)?;
            } else {
                self.credentials.put(
                    &self.scope,
                    &key,
                    &json!({"env":secret.env,"headers":secret.headers}).to_string(),
                )?;
            }
        }
        if let Err(error) = self.persist(configs) {
            if changed {
                match previous {
                    Some(secret) => self.credentials.put(&self.scope, &key, &secret)?,
                    None => self.credentials.delete(&self.scope, &key)?,
                }
            }
            return Err(error);
        }
        self.stop(&input.config.id).await;
        let id = input.config.id.clone();
        self.entries
            .lock()
            .unwrap()
            .insert(id.clone(), Self::entry(input.config));
        self.launch(&id);
        Ok(self.list())
    }
    pub async fn delete(self: &Arc<Self>, id: &str) -> Result<Vec<McpServerView>, String> {
        let _edit = self.edits.lock().await;
        let configs = self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.config.id != id)
            .map(|e| e.config.clone())
            .collect();
        self.persist(configs)?;
        self.stop(id).await;
        self.entries.lock().unwrap().remove(id);
        self.credentials
            .delete(&self.scope, &self.credential_key(id))?;
        Ok(self.list())
    }
    pub async fn shutdown(&self) {
        let ids = self
            .entries
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.stop(&id).await;
        }
    }
    pub async fn set_paused(self: &Arc<Self>, paused: bool) -> Result<(), String> {
        let _edit = self.edits.lock().await;
        if self.paused.swap(paused, Ordering::SeqCst) == paused {
            return Ok(());
        }
        if paused {
            self.shutdown().await;
        } else {
            let ids = {
                let mut entries = self.entries.lock().unwrap();
                entries
                    .values_mut()
                    .for_each(|entry| entry.stop = Cancellation::default());
                entries.keys().cloned().collect::<Vec<_>>()
            };
            for id in ids {
                self.launch(&id);
            }
        }
        Ok(())
    }
    async fn supervise(
        self: Arc<Self>,
        config: McpServerConfig,
        stop: Cancellation,
        state: Arc<Mutex<(String, usize)>>,
    ) {
        for attempt in 0..4 {
            if stop.is_cancelled() {
                break;
            }
            *state.lock().unwrap() = ("connecting".into(), 0);
            let owner = format!("mcp.{}.{}", config.id, attempt);
            let generation = Cancellation::default();
            let mut child = None;
            let startup_deadline = tokio::time::Instant::now() + STARTUP;
            let connect = async {
                let credentials = self.get_credentials(&config.id)?;
                let service: RunningService<RoleClient, ()> = match &config.transport {
                    McpTransport::Stdio { command, args } => {
                        let mut command = Command::new(command);
                        command.args(args).env_clear();
                        for key in ["PATH", "SystemRoot", "TEMP", "TMP"] {
                            if let Some(value) = std::env::var_os(key) {
                                command.env(key, value);
                            }
                        }
                        command
                            .envs(credentials.env)
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .kill_on_drop(true);
                        #[cfg(windows)]
                        {
                            command.creation_flags(0x08000000);
                        }
                        child = Some(command.spawn().map_err(|_| "MCP process failed")?);
                        let process = child.as_mut().unwrap();
                        let stdout = process.stdout.take().ok_or("Missing MCP stdout")?;
                        let stdin = process.stdin.take().ok_or("Missing MCP stdin")?;
                        ().serve(AsyncRwTransport::new(stdout, stdin).with_max_line_length(FRAME))
                            .await
                            .map_err(|_| "MCP initialization failed")?
                    }
                    McpTransport::StreamableHttp { url } => {
                        self.policy.require_safe_transport(url)?;
                        let mut headers = reqwest::header::HeaderMap::new();
                        for (k, v) in credentials.headers {
                            let mut value = reqwest::header::HeaderValue::from_str(&v)
                                .map_err(|_| "Invalid MCP header")?;
                            value.set_sensitive(true);
                            headers.insert(
                                reqwest::header::HeaderName::from_bytes(k.as_bytes())
                                    .map_err(|_| "Invalid MCP header")?,
                                value,
                            );
                        }
                        let client = reqwest::Client::builder()
                            .redirect(reqwest::redirect::Policy::none())
                            .retry(reqwest::retry::never())
                            .timeout(CALL)
                            .default_headers(headers)
                            .build()
                            .map_err(|_| "MCP HTTP client failed")?;
                        let transport = StreamableHttpClientTransport::with_client(
                            client,
                            StreamableHttpClientTransportConfig::with_uri(url.clone())
                                .max_sse_event_size(FRAME)
                                .reinit_on_expired_session(false),
                        );
                        ().serve(transport)
                            .await
                            .map_err(|_| "MCP initialization failed")?
                    }
                };
                Ok::<_, String>(service)
            };
            let connected = tokio::select! { result=tokio::time::timeout_at(startup_deadline,connect)=>result.ok().and_then(Result::ok), _=stop.cancelled()=>None };
            if let Some(mut service) = connected {
                let peer = service.peer().clone();
                let discover = async {
                    let mut tools = Vec::new();
                    let mut cursor = None;
                    for _ in 0..16 {
                        let page = peer
                            .list_tools(Some(
                                rmcp::model::PaginatedRequestParams::default().with_cursor(cursor),
                            ))
                            .await
                            .map_err(|_| "MCP discovery failed")?;
                        if tools.len() + page.tools.len() > 256 {
                            return Err("MCP tool limit exceeded");
                        }
                        tools.extend(page.tools);
                        cursor = page.next_cursor;
                        if cursor.is_none() {
                            return Ok(tools);
                        }
                    }
                    Err("MCP page limit exceeded")
                };
                let tools = tokio::select! { result=tokio::time::timeout_at(startup_deadline,discover)=>result.ok().and_then(Result::ok), _=stop.cancelled()=>None };
                let admitted=tools.map(|tools| {
                    let count=tools.len();
                    for tool in tools {
                        let name=tool.name.to_string(); let remote=name.clone(); let peer=peer.clone(); let generation=generation.clone();
                        let output=tool.output_schema.map(|s|json!({"type":"object","required":["structuredContent"],"properties":{"structuredContent":s}})).unwrap_or(json!({"type":"object"}));
                        self.registry.register_with_timeout(&owner,ToolDefinition { name:format!("mcp.{}.{}",config.id,name), description:tool.description.unwrap_or_default().into_owned(), input_schema:json!(tool.input_schema), output_schema:output, side_effect:true },CALL,move |ctx,args| {
                            let peer=peer.clone(); let remote=remote.clone(); let generation=generation.clone(); Box::pin(async move {
                                if generation.is_cancelled() || ctx.cancellation.is_cancelled() { return Err("MCP connection cancelled".into()); }
                                let args=args.as_object().cloned().ok_or("MCP arguments must be an object")?;
                                let response=tokio::select! { result=tokio::time::timeout(CALL,peer.call_tool_once(CallToolRequestParams::new(remote).with_arguments(args)))=>result.ok().and_then(Result::ok), _=ctx.cancellation.cancelled()=>None, _=generation.cancelled()=>None };
                                match response {
                                    Some(CallToolResponse::Complete(result)) => { let error=result.is_error.unwrap_or(false); let value=serde_json::to_value(result).map_err(|_|"Invalid MCP result")?; if error { Err(value.to_string()) } else { Ok(value) } }
                                    _ => { generation.cancel(); Err("MCP call interrupted or connection failed; outcome may be unknown; call was not replayed".into()) }
                                }
                            })
                        })?;
                    }
                    Ok::<usize,String>(count)
                });
                if let Some(Ok(count)) = admitted {
                    *state.lock().unwrap() = ("connected".into(), count);
                    loop {
                        tokio::select! { _=stop.cancelled()=>break, _=generation.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>if service.is_closed() || peer.is_transport_closed() {break;} }
                    }
                }
                self.registry.revoke_owner(&owner);
                generation.cancel();
                let _ = service.close_with_timeout(CLOSE).await;
            }
            self.registry.revoke_owner(&owner);
            generation.cancel();
            if let Some(mut child) = child {
                let _ = child.kill().await;
                let _ = tokio::time::timeout(CLOSE, child.wait()).await;
            }
            *state.lock().unwrap() = ("disconnected".into(), 0);
            if attempt < 3 {
                tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(250*(1<<attempt)))=>{} }
            }
        }
        *state.lock().unwrap() = ("disconnected".into(), 0);
    }
}

#[cfg(test)]
mod tests;

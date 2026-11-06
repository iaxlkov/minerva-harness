//! Codex integration owns the app-server process boundary; it is not a model provider.
use cordis::{Context, Plugin, Service};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{convert::Infallible, process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// GUI apps on macOS may not inherit the terminal PATH. Also look for
// the CLI bundled with ChatGPT, without reading or copying credentials.
fn app_server_command() -> Result<tokio::process::Command, String> {
    let path = resolve_codex(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .ok_or("Не найден Codex. Установите Codex CLI или ChatGPT и перезапустите приложение.")?;
    Ok(tokio::process::Command::new(path))
}

fn resolve_codex(
    path: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let executable = if cfg!(windows) { "codex.exe" } else { "codex" };
    #[allow(unused_mut)] // macOS app bundles add candidates below.
    let mut candidates: Vec<std::path::PathBuf> = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(executable))
        .collect();
    #[cfg(target_os = "macos")]
    {
        let bundled = "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
        candidates.push(std::path::Path::new("/Applications").join(bundled));
        if let Some(home) = home {
            candidates.push(
                std::path::Path::new(home)
                    .join("Applications")
                    .join(bundled),
            );
            candidates.push(std::path::Path::new(home).join(".cargo/bin/codex"));
        }
        candidates.push("/opt/homebrew/bin/codex".into());
        candidates.push("/usr/local/bin/codex".into());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = home;
    candidates.into_iter().find(|candidate| {
        if !candidate.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            candidate
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct LoginStatus {
    pub state: String,
    pub message: String,
}
#[derive(Default)]
pub struct CodexService {
    hosted_unavailable: bool,
    login: Arc<tokio::sync::Mutex<Option<LoginStatus>>>,
    login_task: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
}
impl Drop for CodexService {
    fn drop(&mut self) {
        if let Ok(task) = self.login_task.get_mut() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
    }
}
impl Service for CodexService {
    const NAME: &'static str = "studio.codex";
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct CodexStatus {
    pub connected: bool,
    pub account_type: Option<String>,
    pub email: Option<String>,
    pub message: String,
    pub quota: Option<Vec<QuotaWindow>>,
    pub quota_error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct QuotaWindow {
    pub used_percent: f64,
    #[ts(type = "number | null")]
    pub window_duration_mins: Option<u64>,
    #[ts(type = "number | null")]
    pub resets_at: Option<u64>,
}
impl CodexService {
    pub fn hosted_unavailable() -> Self {
        {
            let mut service = Self::default();
            service.hosted_unavailable = true;
            service
        }
    }
    fn require_local(&self) -> Result<(), String> {
        if self.hosted_unavailable {
            Err(
                "Hosted Codex unavailable until authenticated isolated-home broker is configured"
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    pub async fn login_status(&self) -> Option<LoginStatus> {
        self.login.lock().await.clone()
    }
    pub async fn login(&self) -> Result<LoginStatus, String> {
        self.require_local()?;
        let mut state = self.login.lock().await;
        if state.as_ref().is_some_and(|s| s.state == "pending") {
            return Err("Вход уже выполняется в браузере.".into());
        }
        let mut command = app_server_command()?;
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Не удалось запустить App Server: {e}"))?;
        let mut input = child.stdin.take().ok_or("Нет канала App Server")?;
        let mut lines = BufReader::new(child.stdout.take().ok_or("Нет ответа App Server")?).lines();
        let result=tokio::time::timeout(Duration::from_secs(20),async {
            input.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"oe_rig","title":"OE Rig","version":"0.1.0"}}})).as_bytes()).await.map_err(|e|e.to_string())?;
            loop { let line=lines.next_line().await.map_err(|e|e.to_string())?.ok_or("App Server завершил работу")?; let v:Value=serde_json::from_str(&line).map_err(|_|"Некорректный ответ App Server")?; if v["id"]==1 {if v.get("error").is_some(){return Err("Ошибка инициализации App Server".into());}break;} }
            input.write_all(format!("{}\n{}\n",json!({"method":"initialized","params":{}}),json!({"id":2,"method":"account/login/start","params":{"type":"chatgpt"}})).as_bytes()).await.map_err(|e|e.to_string())?;
            loop {let line=lines.next_line().await.map_err(|e|e.to_string())?.ok_or("App Server завершил работу")?; let v:Value=serde_json::from_str(&line).map_err(|_|"Некорректный ответ App Server")?; if v["id"]==2 {if v.get("error").is_some(){return Err("App Server не смог начать вход.".into());}break Ok::<_,String>(v["result"].clone());}}
        }).await.map_err(|_|"App Server не ответил вовремя.".to_string())??;
        let auth_url = result["authUrl"]
            .as_str()
            .ok_or("App Server не вернул адрес входа.")?;
        let parsed = url::Url::parse(auth_url).map_err(|_| "Некорректный адрес входа")?;
        if parsed.scheme() != "https"
            || !matches!(parsed.host_str(), Some("auth.openai.com" | "chatgpt.com"))
        {
            return Err("App Server вернул неподдерживаемый адрес входа.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let operation: Vec<u16> = std::ffi::OsStr::new("open")
                .encode_wide()
                .chain(Some(0))
                .collect();
            let address: Vec<u16> = std::ffi::OsStr::new(auth_url)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let result = unsafe {
                windows_sys::Win32::UI::Shell::ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    address.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                )
            };
            if result as isize <= 32 {
                return Err("Не удалось открыть браузер для входа.".into());
            }
        }
        #[cfg(not(windows))]
        {
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let status = tokio::process::Command::new(opener)
                .arg(auth_url)
                .status()
                .await
                .map_err(|e| format!("Не удалось открыть браузер для входа: {e}"))?;
            if !status.success() {
                return Err("Не удалось открыть браузер для входа.".into());
            }
        }
        let pending = LoginStatus {
            state: "pending".into(),
            message: "Завершите вход в открывшемся браузере.".into(),
        };
        *state = Some(pending.clone());
        let shared = self.login.clone();
        let task = tokio::spawn(async move {
            // Keep stdin open while waiting for the browser callback.
            let _input = input;
            let completed = tokio::time::timeout(Duration::from_secs(300), async {
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if v["method"] == "account/login/completed" {
                            return v["params"]["success"].as_bool().unwrap_or(false);
                        }
                    }
                }
                false
            })
            .await
            .unwrap_or(false);
            *shared.lock().await = Some(if completed {
                LoginStatus {
                    state: "complete".into(),
                    message: "Вход выполнен. Обновите состояние подключения.".into(),
                }
            } else {
                LoginStatus {
                    state: "failed".into(),
                    message: "Вход не завершён. Попробуйте снова.".into(),
                }
            });
            let _ = child.kill().await;
        });
        *self.login_task.lock().map_err(|e| e.to_string())? = Some(task.abort_handle());
        Ok(pending)
    }
    pub async fn status(&self) -> Result<CodexStatus, String> {
        self.inspect(false).await
    }
    pub async fn quota(&self) -> Result<CodexStatus, String> {
        self.inspect(true).await
    }
    async fn inspect(&self, include_quota: bool) -> Result<CodexStatus, String> {
        self.require_local()?;
        let mut command = app_server_command()?;
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Не удалось запустить Codex App Server: {e}"))?;
        let mut input = child.stdin.take().ok_or("Нет канала App Server")?;
        let output = child.stdout.take().ok_or("Нет ответа App Server")?;
        let mut lines = BufReader::new(output).lines();
        let result=tokio::time::timeout(Duration::from_secs(20),async {
            async fn write(input:&mut tokio::process::ChildStdin,value:Value)->Result<(),String>{ input.write_all(format!("{value}\n").as_bytes()).await.map_err(|e|e.to_string()) }
            async fn response(lines:&mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,id:u64)->Result<Value,String>{ while let Some(line)=lines.next_line().await.map_err(|e|e.to_string())? { if let Ok(v)=serde_json::from_str::<Value>(&line) { if v.get("id").and_then(Value::as_u64)==Some(id) { if v.get("error").is_some() { return Err("App Server отклонил запрос.".into()); } return Ok(v["result"].clone()); } } } Err("App Server завершил работу.".into()) }
            write(&mut input,json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"oe_rig","title":"OE Rig","version":"0.1.0"}}})).await?; response(&mut lines,1).await?;
            write(&mut input,json!({"method":"initialized","params":{}})).await?;
            write(&mut input,json!({"id":2,"method":"account/read","params":{"refreshToken":false}})).await?;
            let account=response(&mut lines,2).await?; let item=&account["account"];
            let mut quota=None; let mut quota_error=None;
            if include_quota {
                write(&mut input,json!({"id":3,"method":"account/rateLimits/read"})).await?;
                match response(&mut lines,3).await {
                    Ok(value)=>{let limits=&value["rateLimits"];let mut windows=Vec::new();for name in ["primary","secondary"] {let row=&limits[name];if let Some(used)=row["usedPercent"].as_f64(){windows.push(QuotaWindow { used_percent:used,window_duration_mins:row["windowDurationMins"].as_u64(),resets_at:row["resetsAt"].as_u64() });}} quota=Some(windows);},
                    Err(_)=>quota_error=Some("Квота недоступна для этого аккаунта.".to_string()),
                }
            }
            Ok::<_,String>(CodexStatus { connected:true, account_type:item["type"].as_str().map(str::to_owned),email:item["email"].as_str().map(str::to_owned),message:if item.is_null(){"App Server подключён. Учётная запись не найдена."}else{"App Server подключён."}.into(), quota, quota_error })
        }).await.map_err(|_|"App Server не ответил вовремя.".to_string())?;
        let _ = child.kill().await;
        result
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct CodexModel {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct CodexReply {
    #[serde(skip)]
    pub transcript: crate::transcript::Transcript,
    pub thread_id: String,
    pub text: String,
    pub summary: String,
}
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct CodexOptions {
    pub effort: Option<String>,
    pub summary: Option<String>,
    pub verbosity: Option<String>,
}
impl CodexOptions {
    fn validate(&self) -> Result<(), String> {
        if self.effort.as_deref().is_some_and(|v| {
            !matches!(
                v,
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
            )
        }) {
            return Err("Неизвестный уровень рассуждений.".into());
        }
        if self
            .summary
            .as_deref()
            .is_some_and(|v| !matches!(v, "auto" | "concise" | "detailed" | "none"))
        {
            return Err("Неизвестный формат сводки.".into());
        }
        if self
            .verbosity
            .as_deref()
            .is_some_and(|v| !matches!(v, "low" | "medium" | "high"))
        {
            return Err("Неизвестная детализация ответа.".into());
        }
        Ok(())
    }
}

struct Rpc {
    pending: std::collections::VecDeque<Value>,
    child: Option<tokio::process::Child>,
    input: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    lines: tokio::io::Lines<BufReader<Box<dyn tokio::io::AsyncRead + Send + Unpin>>>,
}
impl Rpc {
    async fn start() -> Result<Self, String> {
        let mut command = app_server_command()?;
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Не удалось запустить Codex App Server: {e}"))?;
        let input = Box::new(child.stdin.take().ok_or("Нет канала App Server")?);
        let output: Box<dyn tokio::io::AsyncRead + Send + Unpin> =
            Box::new(child.stdout.take().ok_or("Нет ответа App Server")?);
        let lines = BufReader::new(output).lines();
        let mut rpc = Self {
            child: Some(child),
            input,
            lines,
            pending: Default::default(),
        };
        rpc.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"oe_rig","title":"OE Rig","version":"0.1.0"}}})).await?;
        rpc.response(1).await?;
        rpc.send(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(rpc)
    }
    async fn send(&mut self, v: Value) -> Result<(), String> {
        self.input
            .write_all(format!("{v}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())
    }
    async fn next(&mut self) -> Result<Value, String> {
        if let Some(value) = self.pending.pop_front() {
            return Ok(value);
        }
        self.read_wire().await
    }
    async fn read_wire(&mut self) -> Result<Value, String> {
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .map_err(|e| e.to_string())?
                .ok_or("App Server завершил работу.")?;
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                return Ok(v);
            }
        }
    }
    async fn response(&mut self, id: u64) -> Result<Value, String> {
        loop {
            let v = self.read_wire().await?;
            if v.get("id").and_then(Value::as_u64) == Some(id) && v.get("method").is_none() {
                if let Some(error) = v.get("error") {
                    return Err(format!(
                        "App Server: {}",
                        error["message"].as_str().unwrap_or("ошибка запроса")
                    ));
                }
                return Ok(v["result"].clone());
            }
            if v.get("id").is_some() && v.get("method").is_some() {
                self.send(json!({"id":v["id"],"error":{"code":-32601,"message":"Request unsupported by client"}})).await?;
            } else if v.get("method").is_some() {
                queue_notification(&mut self.pending, v)?;
            }
        }
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}
impl CodexService {
    pub async fn models(&self) -> Result<Vec<CodexModel>, String> {
        self.require_local()?;
        tokio::time::timeout(Duration::from_secs(30),async {
            let mut rpc=Rpc::start().await?;
            rpc.send(json!({"id":10,"method":"account/read","params":{"refreshToken":false}})).await?;
            let account=rpc.response(10).await?;
            if account["account"].is_null() {return Err("Войдите в ChatGPT в настройках Codex, чтобы загрузить модели подписки.".into());}
            let mut cursor:Option<String>=None; let mut models=Vec::new();
            loop { rpc.send(json!({"id":2,"method":"model/list","params":{"limit":100,"cursor":cursor,"includeHidden":false}})).await?; let page=rpc.response(2).await?;
                for item in page["data"].as_array().ok_or("Некорректный каталог моделей Codex.")? {if item["hidden"].as_bool().unwrap_or(false){continue;}
if let Some(id)=item["model"].as_str().or_else(||item["id"].as_str()){models.push(CodexModel{id:id.into(),name:item["displayName"].as_str().unwrap_or(id).into(),is_default:item["isDefault"].as_bool().unwrap_or(false),reasoning_efforts:item["supportedReasoningEfforts"].as_array().map(|values|values.iter().filter_map(|v|v["reasoningEffort"].as_str().map(str::to_owned)).collect()).unwrap_or_default(),default_reasoning_effort:item["defaultReasoningEffort"].as_str().map(str::to_owned)});}}
                cursor=page["nextCursor"].as_str().map(str::to_owned); if cursor.is_none(){break;}
            } Ok(models)
        }).await.map_err(|_|"Превышено время ожидания каталога Codex.".to_string())?
    }
    #[cfg(test)]
    pub async fn chat(
        &self,
        model: String,
        thread_id: Option<String>,
        messages: Vec<crate::sessions::Turn>,
        options: CodexOptions,
    ) -> Result<CodexReply, String> {
        self.chat_recorded(
            model,
            thread_id,
            messages,
            options,
            std::sync::Arc::new(|_| Ok(())),
        )
        .await
    }
    pub async fn chat_recorded(
        &self,
        model: String,
        thread_id: Option<String>,
        messages: Vec<crate::sessions::Turn>,
        options: CodexOptions,
        sink: crate::transcript::TranscriptSink,
    ) -> Result<CodexReply, String> {
        self.require_local()?;
        options.validate()?;
        let (last, previous) = messages.split_last().ok_or("Сообщение пустое.")?;
        if last.role != "user" || last.text().trim().is_empty() {
            return Err("Нужно сообщение пользователя.".into());
        }
        let prompt = if thread_id.is_some() || previous.is_empty() {
            last.text()
        } else {
            let mut text = String::from("Предыдущий диалог:\n");
            for m in previous {
                if m.role == "user" || m.role == "assistant" {
                    text.push_str(&format!(
                        "{}: {}\n",
                        m.role,
                        serde_json::to_string(&m.parts).map_err(|e| e.to_string())?
                    ));
                }
            }
            text.push_str(&format!("\nНовое сообщение пользователя:\n{}", last.text()));
            text
        };
        tokio::time::timeout(Duration::from_secs(180), async {
            let mut rpc = Rpc::start().await?;
            let method = if thread_id.is_some() { "thread/resume" } else { "thread/start" };
            let mut params = json!({"model":model,"approvalPolicy":"never","sandbox":"read-only"});
            if let Some(id) = thread_id { params["threadId"] = json!(id); }
            if let Some(verbosity) = options.verbosity.as_deref() {
                params["config"] = json!({"model_verbosity":verbosity});
            }
            rpc.send(json!({"id":2,"method":method,"params":params})).await?;
            let thread = rpc.response(2).await?["thread"]["id"].as_str()
                .ok_or("App Server не вернул ID диалога.")?.to_owned();
            let mut params = json!({"threadId":thread,"model":model,"input":[{"type":"text","text":prompt,"text_elements":[]}]});
            if let Some(effort) = options.effort.as_deref() { params["effort"] = json!(effort); }
            if let Some(summary) = options.summary.as_deref() { params["summary"] = json!(summary); }
            rpc.send(json!({"id":3,"method":"turn/start","params":params})).await?;
            let started = rpc.response(3).await?;
            let turn_id = started["turn"]["id"].as_str().ok_or("App Server не вернул ID хода.")?.to_owned();
            let mut text = String::new();
            let mut summary = String::new();
            let mut history = crate::codex_history::CodexHistory::default();
            loop {
                let event = rpc.next().await?;
                if event.get("id").is_none() && !belongs_to_turn(&event, &thread, &turn_id) { continue; }
                match event["method"].as_str() {
                    Some("item/started" | "item/completed") => {
                        history.item(&event["params"]["item"], event["method"] == "item/completed")?;
                        sink(&history.snapshot())?;
                    }
                    Some("item/agentMessage/delta") => {
                        if let Some(delta) = event["params"]["delta"].as_str() { text.push_str(delta); }
                    }
                    Some("item/reasoning/summaryTextDelta") => {
                        if let Some(delta) = event["params"]["delta"].as_str() { summary.push_str(delta); }
                    }
                    Some("turn/completed") => {
                        let turn = &event["params"]["turn"];
                        if let Some(items) = turn["items"].as_array() {
                            for item in items { history.item(item, item["status"] != "inProgress")?; }
                        }
                        sink(&history.snapshot())?;
                        if turn["status"] != "completed" {
                            return Err(format!("Запрос Codex завершился с ошибкой: {}", turn["error"]["message"].as_str().unwrap_or("выполнение прервано")));
                        }
                        let mut transcript = history.snapshot();
                        // Older servers may provide only text deltas. Prefer the
                        // authoritative completed items when they are present.
                        if !transcript.parts.iter().any(|p|p.kind=="text") && !text.is_empty() {
                            transcript.parts.push(crate::sessions::ContentPart::text("text", &text));
                        }
                        if !transcript.parts.iter().any(|p|p.kind=="reasoning") && !summary.is_empty() {
                            transcript.parts.insert(0, crate::sessions::ContentPart::text("reasoning", &summary));
                        }
                        if transcript.parts.is_empty() { return Err("Codex не вернул содержимое ответа.".into()); }
                        sink(&transcript)?;
                        return Ok(CodexReply { thread_id:thread, text:transcript.text(), summary, transcript });
                    }
                    _ if event.get("id").is_some() && event.get("method").is_some() => {
                        rpc.send(json!({"id":event["id"],"error":{"code":-32601,"message":"Request unsupported by client"}})).await?;
                    }
                    _ => {}
                }
            }
        }).await.map_err(|_| "Превышено время ожидания ответа Codex.".to_string())?
    }
}
pub struct CodexPlugin(pub Arc<CodexService>);
impl Plugin for CodexPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _publication = ctx.provide(self.0.clone())?;
        Ok(())
    }
}

fn queue_notification(
    queue: &mut std::collections::VecDeque<Value>,
    value: Value,
) -> Result<(), String> {
    if queue.len() >= 4096 {
        return Err("Слишком много уведомлений App Server до ответа RPC.".into());
    }
    queue.push_back(value);
    Ok(())
}
fn belongs_to_turn(value: &Value, thread: &str, turn: &str) -> bool {
    let params = &value["params"];
    params["threadId"].as_str() == Some(thread)
        && params["turnId"]
            .as_str()
            .or_else(|| params["turn"]["id"].as_str())
            == Some(turn)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn hosted_capability_never_starts_ambient_codex() {
        let s = CodexService::hosted_unavailable();
        assert!(s.login().await.is_err());
        assert!(s.status().await.is_err());
        assert!(s.models().await.is_err());
        assert!(s.quota().await.is_err());
        assert!(s
            .chat("model".into(), None, vec![], CodexOptions::default())
            .await
            .is_err());
    }
    #[test]
    fn discovers_executable_in_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("cli with spaces");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        std::fs::write(&executable, "test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([&bin]).unwrap();
        assert_eq!(resolve_codex(Some(&path), None), Some(executable));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn discovers_chatgpt_bundle_without_terminal_path() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "test").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        // System installation takes priority if present.
        let found = resolve_codex(None, Some(dir.path().as_os_str())).unwrap();
        assert!(found == executable || found.starts_with("/Applications/ChatGPT.app"));
    }
    #[tokio::test]
    async fn dropping_service_aborts_background_login() {
        let service = CodexService::default();
        let task = tokio::spawn(std::future::pending::<()>());
        *service.login_task.lock().unwrap() = Some(task.abort_handle());
        drop(service);
        assert!(task.await.unwrap_err().is_cancelled());
    }
    #[tokio::test]
    async fn rpc_preserves_notifications_before_response_and_filters_other_turns() {
        let (client, mut server) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(client);
        let input: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(write);
        let output: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(read);
        let mut rpc = Rpc {
            child: None,
            input,
            lines: BufReader::new(output).lines(),
            pending: Default::default(),
        };
        let early = json!({"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"r","delta":"first"}});
        server
            .write_all(
                format!(
                    "{}\n{}\n",
                    early,
                    json!({"id":3,"result":{"turn":{"id":"r"}}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.response(3).await.unwrap()["turn"]["id"], "r");
        let saved = rpc.next().await.unwrap();
        assert_eq!(saved, early);
        assert!(belongs_to_turn(&saved, "t", "r"));
        assert!(!belongs_to_turn(&saved, "t", "other"));
    }
    #[test]
    fn rejects_unknown_codex_options() {
        assert!(CodexOptions {
            effort: Some("unknown".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(CodexOptions {
            summary: Some("unknown".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(CodexOptions {
            verbosity: Some("unknown".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(CodexOptions {
            effort: Some("high".into()),
            summary: Some("concise".into()),
            verbosity: Some("low".into())
        }
        .validate()
        .is_ok());
    }
    #[tokio::test]
    #[ignore = "requires a signed-in Codex and makes two model requests"]
    async fn local_catalog_and_conversation_resume() {
        let service = CodexService::default();
        let models = service.models().await.expect("model catalog");
        let model = models
            .iter()
            .find(|m| m.is_default)
            .or_else(|| models.first())
            .expect("at least one model")
            .id
            .clone();
        let effort = models
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.reasoning_efforts.first())
            .cloned();
        let options = CodexOptions {
            effort,
            summary: Some("concise".into()),
            verbosity: Some("low".into()),
        };
        let reply = service
            .chat(
                model.clone(),
                None,
                vec![crate::sessions::Turn::plain(
                    "user",
                    "Сколько будет 17 плюс 25? Ответь кратко.",
                )],
                options,
            )
            .await
            .expect("first turn with settings");
        assert!(!reply.text.is_empty());
        let continued = service
            .chat(
                model,
                Some(reply.thread_id.clone()),
                vec![crate::sessions::Turn::plain(
                    "user",
                    "Ответь одним словом: да.",
                )],
                CodexOptions::default(),
            )
            .await
            .expect("resume turn");
        assert_eq!(reply.thread_id, continued.thread_id);
        assert!(!continued.text.is_empty());
    }
    #[tokio::test]
    #[ignore = "requires an installed Codex executable and local app-server"]
    async fn local_app_server_handshake() {
        let status = CodexService::default()
            .status()
            .await
            .expect("app-server handshake");
        assert!(status.connected);
    }
}

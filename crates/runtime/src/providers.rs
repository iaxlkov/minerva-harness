//! Model provider configuration and write-only OS credentials.
use cordis::{Context, Plugin, Service};
use rig_agent::AgentBuilder;
use rig_core::providers::{anthropic::AnthropicConfig, openai::OpenAIConfig};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, fs, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
#[ts(rename = "ProviderConfig")]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}
fn default_protocol() -> String {
    "openai-chat".into()
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ProviderView {
    #[serde(flatten)]
    provider: Provider,
    has_key: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ProviderInput {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub api_key: Option<String>,
    pub is_new: bool,
    pub protocol: String,
}
use crate::credential_store::{CredentialScope, CredentialStore, LocalKeyring};
use crate::network_policy::EndpointPolicy;
use crate::sessions::Turn;
pub const PREAMBLE: &str =
    "Ты помощник разработчика. Отвечай на русском, если пользователь не просит иначе.";

pub struct ProviderService {
    path: PathBuf,
    providers: Mutex<Vec<Provider>>,
    scope: CredentialScope,
    credentials: Arc<dyn CredentialStore>,
    endpoint_policy: EndpointPolicy,
}
impl Service for ProviderService {
    const NAME: &'static str = "studio.model-providers";
}
impl ProviderService {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        Self::new_with_store(
            path,
            CredentialScope::new("local")?,
            Arc::new(LocalKeyring),
            EndpointPolicy::Local,
        )
    }
    pub fn new_with_store(
        path: PathBuf,
        scope: CredentialScope,
        credentials: Arc<dyn CredentialStore>,
        endpoint_policy: EndpointPolicy,
    ) -> Result<Self, String> {
        if endpoint_policy == EndpointPolicy::Hosted && credentials.local_environment_fallback() {
            return Err("Hosted runtime requires scoped credential broker".into());
        }
        let providers = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Не удалось прочитать провайдеры: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            path,
            providers: Mutex::new(providers),
            scope,
            credentials,
            endpoint_policy,
        })
    }
    fn env_key(&self, id: &str, endpoint: &str) -> Option<String> {
        if !self.credentials.local_environment_fallback()
            || self.endpoint_policy == EndpointPolicy::Hosted
        {
            return None;
        }
        let expected = match id {
            "openai" => "https://api.openai.com/v1",
            "deepseek" => "https://api.deepseek.com",
            "anthropic" => "https://api.anthropic.com",
            _ => return None,
        };
        if endpoint.trim_end_matches('/') != expected {
            return None;
        }
        let name = match id {
            "openai" => "OPENAI_API_KEY",
            "deepseek" => "DEEPSEEK_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            _ => return None,
        };
        std::env::var(name).ok().filter(|s| !s.trim().is_empty())
    }
    fn has_key(&self, p: &Provider) -> bool {
        self.credentials
            .get(&self.scope, &p.id)
            .ok()
            .flatten()
            .is_some()
            || self.env_key(&p.id, &p.base_url).is_some()
    }
    pub async fn list(&self) -> Vec<ProviderView> {
        self.providers
            .lock()
            .await
            .iter()
            .cloned()
            .map(|p| ProviderView {
                has_key: self.has_key(&p),
                provider: p,
            })
            .collect()
    }
    pub async fn save(&self, input: ProviderInput) -> Result<Vec<ProviderView>, String> {
        if input.id.is_empty()
            || input.id.len() > 64
            || !input
                .id
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
            || !input
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err("ID должен начинаться с латинской буквы и содержать строчные буквы, цифры, - или _.".into());
        }
        if !matches!(
            input.protocol.as_str(),
            "openai-chat" | "openai-responses" | "anthropic-messages"
        ) {
            return Err("Неизвестный протокол API.".into());
        }
        if input.name.trim().is_empty()
            || input.models.is_empty()
            || input.models.iter().any(|m| m.trim().is_empty())
        {
            return Err("Укажите название и хотя бы одну модель.".into());
        }
        self.endpoint_policy.validate(&input.base_url)?;

        let mut providers = self.providers.lock().await;
        if input.is_new && providers.iter().any(|p| p.id == input.id) {
            return Err("Провайдер с таким ID уже существует.".into());
        }
        if !input.is_new && !providers.iter().any(|p| p.id == input.id) {
            return Err("Провайдер не найден; обновите список.".into());
        }
        if let Some(existing) = providers.iter().find(|p| p.id == input.id) {
            if (existing.base_url.trim_end_matches('/') != input.base_url.trim_end_matches('/')
                || existing.protocol != input.protocol)
                && !input
                    .api_key
                    .as_ref()
                    .is_some_and(|key| !key.trim().is_empty())
            {
                return Err("При смене адреса или протокола введите ключ явно.".into());
            }
        }
        let mut next = providers.clone();
        let provider = Provider {
            id: input.id.clone(),
            name: input.name.trim().into(),
            base_url: input.base_url.trim_end_matches('/').into(),
            protocol: input.protocol,
            models: input
                .models
                .into_iter()
                .map(|x| x.trim().to_string())
                .collect(),
        };
        if let Some(existing) = next.iter_mut().find(|x| x.id == provider.id) {
            *existing = provider;
        } else {
            next.push(provider);
        }
        let dir = self.path.parent().ok_or("Не задан каталог настроек")?;
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut temp, &next).map_err(|e| e.to_string())?;
        let changed_key = input.api_key.filter(|k| !k.trim().is_empty());
        let old_key = if changed_key.is_some() {
            self.credentials.get(&self.scope, &input.id)?
        } else {
            None
        };
        if let Some(key) = changed_key.as_ref() {
            if let Err(e) = self.credentials.put(&self.scope, &input.id, key) {
                return Err(format!("Не удалось сохранить ключ: {e}"));
            }
        }
        if let Err(e) = temp.persist(&self.path) {
            if changed_key.is_some() {
                let rollback = if let Some(old) = old_key {
                    self.credentials.put(&self.scope, &input.id, &old)
                } else {
                    self.credentials.delete(&self.scope, &input.id)
                };
                if rollback.is_err() {
                    return Err(format!("Не удалось сохранить настройки: {e}; также не удалось восстановить прежний ключ."));
                }
            }
            return Err(format!("Не удалось сохранить настройки: {e}"));
        }
        *providers = next;
        drop(providers);
        Ok(self.list().await)
    }
    pub async fn delete(&self, id: &str) -> Result<Vec<ProviderView>, String> {
        let mut providers = self.providers.lock().await;
        let mut next = providers.clone();
        next.retain(|p| p.id != id);
        let dir = self.path.parent().ok_or("Не задан каталог настроек")?;
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut temp, &next).map_err(|e| e.to_string())?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        *providers = next;
        drop(providers);
        self.credentials.delete(&self.scope, id)?;
        Ok(self.list().await)
    }
    pub async fn discover(&self, input: ProviderInput) -> Result<Vec<String>, String> {
        self.endpoint_policy
            .require_safe_transport(&input.base_url)?;

        let key = {
            let saved = self.providers.lock().await;
            let matches_saved = saved
                .iter()
                .any(|p| p.id == input.id && same_endpoint(p, &input));
            input
                .api_key
                .as_ref()
                .filter(|k| !k.trim().is_empty())
                .cloned()
                .or_else(|| {
                    if matches_saved {
                        self.credentials.get(&self.scope, &input.id).ok().flatten()
                    } else {
                        None
                    }
                })
                .or_else(|| self.env_key(&input.id, &input.base_url))
                .ok_or("Введите API-ключ для указанного адреса.")?
        };
        let request = async {
            match input.protocol.as_str() {
                "openai-chat" | "openai-responses" => {
                    OpenAIConfig::new(key)
                        .with_base_url(input.base_url)
                        .client()
                        .list_models()
                        .await
                }
                "anthropic-messages" => {
                    AnthropicConfig::new(key)
                        .with_base_url(input.base_url)
                        .client()
                        .list_models()
                        .await
                }
                _ => return Err("Неизвестный протокол API.".to_string()),
            }
            .map(|list| list.data.into_iter().map(|m| m.id).collect())
            .map_err(|_| {
                "Не удалось получить список моделей. Проверьте URL, протокол и ключ.".to_string()
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(20), request)
            .await
            .map_err(|_| "Превышено время ожидания списка моделей.".to_string())?
    }
    #[cfg(test)]
    pub async fn chat(&self, id: &str, model: &str, messages: Vec<Turn>) -> Result<String, String> {
        self.chat_recorded(id, model, messages, std::sync::Arc::new(|_| Ok(())))
            .await
            .map(|v| v.text())
    }
    pub async fn chat_recorded(
        &self,
        id: &str,
        model: &str,
        messages: Vec<Turn>,
        sink: crate::transcript::TranscriptSink,
    ) -> Result<crate::transcript::Transcript, String> {
        let (provider, key) = {
            let providers = self.providers.lock().await;
            let provider = providers
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .ok_or("Provider not found")?;
            if !provider.models.iter().any(|m| m == model) {
                return Err("Model not found".into());
            }
            self.endpoint_policy
                .require_safe_transport(&provider.base_url)?;
            let key = self
                .credentials
                .get(&self.scope, id)?
                .or_else(|| self.env_key(id, &provider.base_url))
                .ok_or("API key not configured")?;
            (provider, key)
        };
        let (last, prev) = messages.split_last().ok_or("Сообщение пустое.")?;
        if last.role != "user" || last.text().trim().is_empty() {
            return Err("Нужно сообщение пользователя.".into());
        }
        let history = crate::rig_history::replay(prev)?;
        macro_rules! run_agent {
            ($model:expr) => {{
                let builder = AgentBuilder::new($model).preamble(PREAMBLE);
                let builder = if provider.protocol == "anthropic-messages" {
                    builder.max_tokens(4096)
                } else { builder };
                let agent = builder.build();
                let request = agent.prompt(last.text()).history(history)
                    .add_hook(crate::rig_history::HistoryHook::new(sink.clone()));
                let response = tokio::time::timeout(std::time::Duration::from_secs(90), request)
                    .await.map_err(|_| "Превышено время ожидания модели.")?
                    .map_err(|_| "Не удалось получить ответ от провайдера. Проверьте адрес, ключ и модель.")?;
                // PromptResponse owns only this run's additions: skip its input
                // prompt, retain every accepted model turn and tool result.
                let generated = response.messages.get(1..).ok_or("Rig не вернул историю ответа.")?;
                let transcript = crate::rig_history::capture(generated)?;
                sink(&transcript)?;
                Ok(transcript)
            }};
        }
        match provider.protocol.as_str() {
            "openai-chat" => run_agent!(OpenAIConfig::new(key)
                .with_base_url(provider.base_url)
                .client()
                .chat(model)),
            "openai-responses" => run_agent!(OpenAIConfig::new(key)
                .with_base_url(provider.base_url)
                .client()
                .responses(model)),
            "anthropic-messages" => run_agent!(AnthropicConfig::new(key)
                .with_base_url(provider.base_url)
                .client()
                .completion(model)),
            _ => Err("Неизвестный протокол API.".into()),
        }
    }
}
pub struct ProviderPlugin(pub Arc<ProviderService>);
impl Plugin for ProviderPlugin {
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

fn same_endpoint(saved: &Provider, input: &ProviderInput) -> bool {
    saved.base_url.trim_end_matches('/') == input.base_url.trim_end_matches('/')
        && saved.protocol == input.protocol
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn input(endpoint: &str, key: Option<&str>, is_new: bool) -> ProviderInput {
        ProviderInput {
            id: "openai".into(),
            name: "Test".into(),
            base_url: endpoint.into(),
            models: vec!["test".into()],
            api_key: key.map(str::to_owned),
            is_new,
            protocol: "openai-chat".into(),
        }
    }
    #[tokio::test]
    async fn scoped_provider_crud_and_changed_endpoint_do_not_leak_keys() {
        use crate::credential_store::MemoryCredentialStore;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryCredentialStore::default());
        let a = ProviderService::new_with_store(
            dir.path().join("a.json"),
            CredentialScope::new("a").unwrap(),
            store.clone(),
            EndpointPolicy::Local,
        )
        .unwrap();
        let b = ProviderService::new_with_store(
            dir.path().join("b.json"),
            CredentialScope::new("b").unwrap(),
            store.clone(),
            EndpointPolicy::Local,
        )
        .unwrap();
        a.save(input("https://example.com/v1", Some("alice"), true))
            .await
            .unwrap();
        b.save(input("https://example.com/v1", Some("bob"), true))
            .await
            .unwrap();
        assert!(a.list().await[0].has_key);
        assert!(b.list().await[0].has_key);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        assert!(a.discover(input(&endpoint, None, false)).await.is_err());
        assert!(a.save(input(&endpoint, None, false)).await.is_err());
        assert!(listener.accept().is_err());
        assert_eq!(
            store
                .get(&CredentialScope::new("a").unwrap(), "openai")
                .unwrap()
                .as_deref(),
            Some("alice")
        );
        a.delete("openai").await.unwrap();
        assert!(b.list().await[0].has_key);
        assert_eq!(
            store
                .get(&CredentialScope::new("b").unwrap(), "openai")
                .unwrap()
                .as_deref(),
            Some("bob")
        );
    }
    #[tokio::test]
    async fn hosted_provider_never_touches_ambient_credentials_or_transport() {
        let dir = tempfile::tempdir().unwrap();
        let s = ProviderService::new_with_store(
            dir.path().join("p.json"),
            CredentialScope::new("hosted").unwrap(),
            Arc::new(crate::credential_store::UnavailableCredentialStore),
            EndpointPolicy::Hosted,
        )
        .unwrap();
        assert!(s
            .discover(input("https://api.openai.com/v1", Some("explicit"), true))
            .await
            .unwrap_err()
            .contains("transport unavailable"));
        assert!(ProviderService::new_with_store(
            dir.path().join("q.json"),
            CredentialScope::new("hosted").unwrap(),
            Arc::new(LocalKeyring),
            EndpointPolicy::Hosted
        )
        .is_err());
    }
    #[tokio::test]
    async fn all_protocols_keep_unavailable_tools_as_context_on_their_own_wires() {
        let fixtures = [
            (
                "openai-chat",
                "/v1/chat/completions",
                serde_json::json!({"id":"r","object":"chat.completion","created":1,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"wire reply"},"finish_reason":"stop"}]}),
            ),
            (
                "openai-responses",
                "/v1/responses",
                serde_json::json!({"id":"r","object":"response","created_at":0,"status":"completed","model":"test-model","output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"text":"wire reply"}]}],"tools":[]}),
            ),
            (
                "anthropic-messages",
                "/v1/messages",
                serde_json::json!({"id":"m","type":"message","model":"test-model","role":"assistant","stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1},"content":[{"type":"text","text":"wire reply"}]}),
            ),
        ];
        for (protocol, route, body) in fixtures {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                listener.set_nonblocking(true).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(value) => break value,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(10))
                        }
                        Err(error) => panic!("no provider request: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(pos) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..pos]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= pos + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&data);
                assert!(
                    request.starts_with(&format!("POST {route} ")),
                    "wrong route"
                );
                for text in [
                    "previous question",
                    "previous answer",
                    "current question",
                    "lookup",
                    "observed tool result",
                ] {
                    assert!(
                        request.contains(text),
                        "{protocol} missing {text}: {}",
                        request.split("\r\n\r\n").nth(1).unwrap_or_default()
                    );
                }
                let body = body.to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let service = ProviderService::new(dir.path().join("providers.json")).unwrap();
            let id = format!("wiretest{}{}", std::process::id(), protocol);
            let base_url = if protocol == "anthropic-messages" {
                format!("http://{address}")
            } else {
                format!("http://{address}/v1")
            };
            service
                .save(ProviderInput {
                    id: id.clone(),
                    name: "Test".into(),
                    base_url,
                    models: vec!["test-model".into()],
                    api_key: Some("local-only-test-key".into()),
                    is_new: true,
                    protocol: protocol.into(),
                })
                .await
                .unwrap();
            let answer = service
                .chat(
                    &id,
                    "test-model",
                    vec![
                        crate::sessions::Turn::plain("user", "previous question"),
                        crate::sessions::Turn {
                            role: "assistant".into(),
                            parts: vec![
                                crate::sessions::ContentPart::call(crate::transcript::ToolCall {
                                    call_id: "call_123".into(),
                                    name: "lookup".into(),
                                    arguments: serde_json::json!({"id":1}),
                                }),
                                crate::sessions::ContentPart::result(
                                    crate::transcript::ToolResult {
                                        call_id: "call_123".into(),
                                        output: serde_json::json!({"value":"observed tool result"}),
                                        is_error: false,
                                    },
                                ),
                                crate::sessions::ContentPart::text("text", "previous answer"),
                            ],
                            replay: serde_json::Map::new(),
                        },
                        crate::sessions::Turn::plain("user", "current question"),
                    ],
                )
                .await;
            service.delete(&id).await.unwrap();
            assert_eq!(answer.unwrap(), "wire reply");
            server.join().unwrap();
        }
    }
    #[tokio::test]
    async fn provider_credentials_stay_out_of_config_and_reach_only_local_mock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = socket.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(pos) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..pos]);
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= pos + 4 + len {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&data);
            assert!(text.contains("test-model"));
            assert!(text.contains("test-key-only"));
            let body = r#"{"id":"test","object":"chat.completion","created":1,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"local reply"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let service = ProviderService::new(path.clone()).unwrap();
        let id = format!("testrig{}", std::process::id());
        service
            .save(ProviderInput {
                id: id.clone(),
                name: "Test".into(),
                base_url: format!("http://{address}/v1"),
                models: vec!["test-model".into()],
                api_key: Some("test-key-only".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let stored = fs::read_to_string(path).unwrap();
        assert!(!stored.contains("test-key-only"));
        let answer = service
            .chat(
                &id,
                "test-model",
                vec![crate::sessions::Turn::plain("user", "hello")],
            )
            .await
            .unwrap();
        assert_eq!(answer, "local reply");
        server.join().unwrap();
        assert!(service.chat(&id, "unknown", vec![]).await.is_err());
        let changed = || ProviderInput {
            id: id.clone(),
            name: "Test".into(),
            base_url: "http://127.0.0.1:1/v1".into(),
            models: vec!["test-model".into()],
            api_key: None,
            is_new: false,
            protocol: "openai-chat".into(),
        };
        assert!(service
            .save(changed())
            .await
            .err()
            .unwrap()
            .contains("ключ"));
        assert!(service
            .discover(changed())
            .await
            .unwrap_err()
            .contains("ключ"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).contains("/v1/models"));
            let body = r#"{"object":"list","data":[{"id":"listed-model","object":"model","created":1,"owned_by":"test"}]}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let found = service
            .discover(ProviderInput {
                id: id.clone(),
                name: "Test".into(),
                base_url: format!("http://{address}/v1"),
                models: vec![],
                api_key: Some("test-key-only".into()),
                is_new: false,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        assert_eq!(found, vec!["listed-model"]);
        server.join().unwrap();
        service.delete(&id).await.unwrap();
    }
}

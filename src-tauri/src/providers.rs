//! Model provider configuration and write-only OS credentials.
use std::{convert::Infallible, fs, path::PathBuf, sync::Arc};
use cordis::{Context, Plugin, Service};
use rig_agent::AgentBuilder;
use rig_core::{completion::Message, providers::{openai::OpenAIConfig,anthropic::AnthropicConfig}};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Provider { pub id:String, pub name:String, pub base_url:String, pub models:Vec<String>, #[serde(default="default_protocol")] pub protocol:String }
fn default_protocol()->String { "openai-chat".into() }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct ProviderView { #[serde(flatten)] provider: Provider, has_key: bool }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ProviderInput { pub id:String, pub name:String, pub base_url:String, pub models:Vec<String>, pub api_key:Option<String>, pub is_new:bool, pub protocol:String }
#[derive(Deserialize)]
pub struct Turn { pub role:String, pub text:String }

pub struct ProviderService { path:PathBuf, providers:Mutex<Vec<Provider>> }
impl Service for ProviderService { const NAME:&'static str="studio.model-providers"; }
impl ProviderService {
    pub fn new(path:PathBuf) -> Result<Self,String> {
        let providers=match fs::read(&path) { Ok(bytes)=>serde_json::from_slice(&bytes).map_err(|e| format!("Не удалось прочитать провайдеры: {e}"))?, Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Vec::new(), Err(e)=>return Err(e.to_string()) };
        Ok(Self { path, providers:Mutex::new(providers) })
    }
    fn entry(id:&str)->Result<keyring::Entry,String> { keyring::Entry::new("OE Rig model providers", id).map_err(|e|e.to_string()) }
    fn env_key(id:&str)->Option<String> { let name=match id {"openai"=>"OPENAI_API_KEY", "deepseek"=>"DEEPSEEK_API_KEY", "anthropic"=>"ANTHROPIC_API_KEY", _=>return None}; std::env::var(name).ok().filter(|s|!s.trim().is_empty()) }
    fn has_key(id:&str)->bool { Self::entry(id).ok().and_then(|e|e.get_password().ok()).is_some() || Self::env_key(id).is_some() }
    pub async fn list(&self)->Vec<ProviderView> { self.providers.lock().await.iter().cloned().map(|p| ProviderView { has_key:Self::has_key(&p.id), provider:p }).collect() }
    pub async fn save(&self, input:ProviderInput)->Result<Vec<ProviderView>,String> {
        if input.id.is_empty() || input.id.len()>64 || !input.id.chars().next().is_some_and(|c|c.is_ascii_lowercase()) || !input.id.chars().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c=='-'||c=='_') { return Err("ID должен начинаться с латинской буквы и содержать строчные буквы, цифры, - или _.".into()); }
        if !matches!(input.protocol.as_str(), "openai-chat"|"openai-responses"|"anthropic-messages") {return Err("Неизвестный протокол API.".into());}
        if input.name.trim().is_empty() || input.models.is_empty() || input.models.iter().any(|m|m.trim().is_empty()) { return Err("Укажите название и хотя бы одну модель.".into()); }
        let parsed=url::Url::parse(&input.base_url).map_err(|_|"Некорректный URL провайдера.")?;
        if !(parsed.scheme()=="https" || (parsed.scheme()=="http" && matches!(parsed.host_str(),Some("localhost"|"127.0.0.1"|"::1")))) || !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() { return Err("Используйте HTTPS или локальный HTTP-адрес без учётных данных, параметров и фрагмента.".into()); }
        let mut providers=self.providers.lock().await;
        if input.is_new && providers.iter().any(|p|p.id==input.id) { return Err("Провайдер с таким ID уже существует.".into()); }
        if !input.is_new && !providers.iter().any(|p|p.id==input.id) { return Err("Провайдер не найден; обновите список.".into()); }
        let mut next=providers.clone();
        let provider=Provider {id:input.id.clone(),name:input.name.trim().into(),base_url:input.base_url.trim_end_matches('/').into(),protocol:input.protocol,models:input.models.into_iter().map(|x|x.trim().to_string()).collect()};
        if let Some(existing)=next.iter_mut().find(|x|x.id==provider.id) { *existing=provider; } else { next.push(provider); }
        let dir=self.path.parent().ok_or("Не задан каталог настроек")?;
        fs::create_dir_all(dir).map_err(|e|e.to_string())?;
        let mut temp=tempfile::NamedTempFile::new_in(dir).map_err(|e|e.to_string())?;
        serde_json::to_writer_pretty(&mut temp,&next).map_err(|e|e.to_string())?;
        let changed_key=input.api_key.filter(|k|!k.trim().is_empty());
        let old_key=if changed_key.is_some(){Self::entry(&input.id)?.get_password().ok()}else{None};
        if let Some(key)=changed_key.as_ref() { if let Err(e)=Self::entry(&input.id)?.set_password(key) { return Err(format!("Не удалось сохранить ключ: {e}")); } }
        if let Err(e)=temp.persist(&self.path) {
            if changed_key.is_some() { let entry=Self::entry(&input.id)?; let rollback=if let Some(old)=old_key {entry.set_password(&old)}else{entry.delete_credential()}; if rollback.is_err() {return Err(format!("Не удалось сохранить настройки: {e}; также не удалось восстановить прежний ключ."));} }
            return Err(format!("Не удалось сохранить настройки: {e}"));
        }
        *providers=next;
        drop(providers); Ok(self.list().await)
    }
    pub async fn delete(&self,id:&str)->Result<Vec<ProviderView>,String> {
        let mut providers=self.providers.lock().await; let mut next=providers.clone(); next.retain(|p|p.id!=id);
        let dir=self.path.parent().ok_or("Не задан каталог настроек")?; fs::create_dir_all(dir).map_err(|e|e.to_string())?;
        let mut temp=tempfile::NamedTempFile::new_in(dir).map_err(|e|e.to_string())?; serde_json::to_writer_pretty(&mut temp,&next).map_err(|e|e.to_string())?; temp.persist(&self.path).map_err(|e|e.to_string())?;
        *providers=next; drop(providers); match Self::entry(id)?.delete_credential() { Ok(())|Err(keyring::Error::NoEntry)=>{}, Err(e)=>return Err(format!("Провайдер удалён, ключ не удалось удалить: {e}")) }; Ok(self.list().await)
    }
    pub async fn discover(&self,input:ProviderInput)->Result<Vec<String>,String> {
        let parsed=url::Url::parse(&input.base_url).map_err(|_|"Некорректный URL провайдера.")?;
        if !(parsed.scheme()=="https" || (parsed.scheme()=="http" && matches!(parsed.host_str(),Some("localhost"|"127.0.0.1"|"::1")))) || !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() {return Err("Некорректный адрес провайдера.".into());}
        let key=input.api_key.filter(|k|!k.trim().is_empty()).or_else(||Self::entry(&input.id).ok().and_then(|e|e.get_password().ok())).or_else(||Self::env_key(&input.id)).ok_or("Введите API-ключ.")?;
        let request=async { match input.protocol.as_str() {
            "openai-chat"|"openai-responses" => OpenAIConfig::new(key).with_base_url(input.base_url).client().list_models().await,
            "anthropic-messages" => AnthropicConfig::new(key).with_base_url(input.base_url).client().list_models().await,
            _ => return Err("Неизвестный протокол API.".to_string()),
        }.map(|list|list.data.into_iter().map(|m|m.id).collect()).map_err(|_|"Не удалось получить список моделей. Проверьте URL, протокол и ключ.".to_string()) };
        tokio::time::timeout(std::time::Duration::from_secs(20),request).await.map_err(|_|"Превышено время ожидания списка моделей.".to_string())?
    }
    pub async fn chat(&self,id:&str,model:&str,messages:Vec<Turn>)->Result<String,String> {
        let provider=self.providers.lock().await.iter().find(|p|p.id==id).cloned().ok_or("Провайдер не найден.")?;
        if !provider.models.iter().any(|m|m==model) { return Err("Модель не найдена у провайдера.".into()); }
        let key=Self::entry(id)?.get_password().ok().or_else(||Self::env_key(id)).ok_or("API-ключ не задан.")?;
        let (last,prev)=messages.split_last().ok_or("Сообщение пустое.")?; if last.role!="user" || last.text.trim().is_empty() {return Err("Нужно сообщение пользователя.".into());}
        let mut history=Vec::new(); for m in prev { match m.role.as_str() {"user"=>history.push(Message::user(&m.text)),"assistant"=>history.push(Message::assistant(&m.text)),_=>return Err("Неизвестная роль.".into())} }
        macro_rules! run_agent { ($model:expr) => {{
            let agent=AgentBuilder::new($model).preamble("Ты помощник разработчика. Отвечай на русском, если пользователь не просит иначе.").build();
            let response=tokio::time::timeout(std::time::Duration::from_secs(90),agent.chat(&last.text,&mut history)).await.map_err(|_|"Превышено время ожидания модели.")?.map_err(|_|"Не удалось получить ответ от провайдера. Проверьте адрес, ключ и модель.")?;
            Ok(response.output())
        }}; }
        match provider.protocol.as_str() {
            "openai-chat" => run_agent!(OpenAIConfig::new(key).with_base_url(provider.base_url).client().chat(model)),
            "openai-responses" => run_agent!(OpenAIConfig::new(key).with_base_url(provider.base_url).client().responses(model)),
            "anthropic-messages" => run_agent!(AnthropicConfig::new(key).with_base_url(provider.base_url).client().completion(model)),
            _ => Err("Неизвестный протокол API.".into()),
        }
    }
}
pub struct ProviderPlugin(pub Arc<ProviderService>);
impl Plugin for ProviderPlugin { type Config=(); type Input=(); type PrepareError=Infallible; type ApplyError=cordis::service::ServicePublishError; fn prepare(&self,():())->Result<(),Infallible>{Ok(())} async fn apply(&self,ctx:Context,_:&())->Result<(),Self::ApplyError>{let _publication=ctx.provide(self.0.clone())?;Ok(())} }

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read,Write};
    #[tokio::test]
    async fn provider_credentials_stay_out_of_config_and_reach_only_local_mock() {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("providers.json");
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let address=listener.local_addr().unwrap();
        let server=std::thread::spawn(move || {
            let (mut socket,_)=listener.accept().unwrap(); socket.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            let mut data=Vec::new(); let mut buf=[0u8;2048]; loop { let n=socket.read(&mut buf).unwrap(); if n==0{break;} data.extend_from_slice(&buf[..n]); if let Some(pos)=data.windows(4).position(|v|v==b"\r\n\r\n") { let head=String::from_utf8_lossy(&data[..pos]); let len=head.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0); if data.len()>=pos+4+len{break;} } }
            let text=String::from_utf8_lossy(&data); assert!(text.contains("test-model")); assert!(text.contains("test-key-only"));
            let body=r#"{"id":"test","object":"chat.completion","created":1,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"local reply"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let service=ProviderService::new(path.clone()).unwrap(); let id=format!("testrig{}",std::process::id());
        service.save(ProviderInput{id:id.clone(),name:"Test".into(),base_url:format!("http://{address}/v1"),models:vec!["test-model".into()],api_key:Some("test-key-only".into()),is_new:true,protocol:"openai-chat".into()}).await.unwrap();
        let stored=fs::read_to_string(path).unwrap();assert!(!stored.contains("test-key-only"));
        let answer=service.chat(&id,"test-model",vec![Turn{role:"user".into(),text:"hello".into()}]).await.unwrap();assert_eq!(answer,"local reply");server.join().unwrap();
        assert!(service.chat(&id,"unknown",vec![]).await.is_err());
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let server=std::thread::spawn(move || {let (mut socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();let mut buf=[0u8;2048];let n=socket.read(&mut buf).unwrap();assert!(String::from_utf8_lossy(&buf[..n]).contains("/v1/models"));let body=r#"{"object":"list","data":[{"id":"listed-model","object":"model","created":1,"owned_by":"test"}]}"#;write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();});
        let found=service.discover(ProviderInput{id:id.clone(),name:"Test".into(),base_url:format!("http://{address}/v1"),models:vec![],api_key:None,is_new:false,protocol:"openai-chat".into()}).await.unwrap();
        assert_eq!(found,vec!["listed-model"]);server.join().unwrap();service.delete(&id).await.unwrap();
    }
}

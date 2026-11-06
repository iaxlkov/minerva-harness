use futures_util::{SinkExt, StreamExt};
use minerva_api::{local_router, LocalApiConfig};
use minerva_runtime::{Runtime, RuntimeConfig};
use reqwest::cookie::CookieStore;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

struct Host {
    base: String,
    token: String,
    client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    runtime: Arc<Runtime>,
    jar: Arc<reqwest::cookie::Jar>,
    _dir: tempfile::TempDir,
}
impl Host {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Arc::new(
            Runtime::start(RuntimeConfig {
                tenant_id: "local".into(),
                workspace_id: "default".into(),
                data_dir: dir.path().join("data"),
                config_dir: dir.path().join("config"),
                profile: "local".into(),
            })
            .await
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let config = LocalApiConfig::new(&base, dir.path().join("dist")).unwrap();
        let token = config.launch_token.clone();
        let router = local_router(runtime.clone(), config);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let jar = Arc::new(reqwest::cookie::Jar::default());
        Self {
            base,
            token,
            task,
            runtime,
            _dir: dir,
            client: reqwest::Client::builder()
                .cookie_provider(jar.clone())
                .build()
                .unwrap(),
            jar,
        }
    }
    async fn login(&self) -> String {
        let response = self
            .client
            .post(format!("{}/api/v1/auth/exchange", self.base))
            .header("Origin", &self.base)
            .json(&json!({"token":self.token}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 204);
        let bootstrap: Value = self
            .client
            .get(format!("{}/api/v1/bootstrap", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(bootstrap["protocolVersion"], 1);
        assert_eq!(bootstrap["workspace"]["id"], "default");
        bootstrap["csrfToken"].as_str().unwrap().to_owned()
    }
    async fn rpc(&self, csrf: &str, method: &str, params: Value) -> Value {
        self.client
            .post(format!("{}/api/v1/rpc", self.base))
            .header("Origin", &self.base)
            .header("X-Minerva-CSRF", csrf)
            .json(&json!({"protocolVersion":1,"method":method,"params":params}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    async fn close(self) {
        self.task.abort();
        let _ = self.runtime.shutdown().await;
    }
}

#[tokio::test]
async fn websocket_reconnect_replaces_baseline_and_logout_revokes_open_stream() {
    let host = Host::start().await;
    let csrf = host.login().await;
    host.rpc(&csrf, "session_create", json!({"id":"one"})).await;
    let ws_url = format!("{}/api/v1/streams", host.base.replace("http://", "ws://"));
    let request = || {
        let mut request = ws_url.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", host.base.parse().unwrap());
        request.headers_mut().insert(
            "cookie",
            host.jar
                .cookies(&format!("{}/api/v1/bootstrap", host.base).parse().unwrap())
                .unwrap(),
        );
        request
    };
    let (mut socket, _) = connect_async(request()).await.unwrap();
    socket
        .send(Message::Text(
            json!({"kind":"open","id":"first","sessionId":"one"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let frame: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(frame["kind"], "baseline");
    assert_eq!(frame["snapshot"]["messages"].as_array().unwrap().len(), 0);
    socket.close(None).await.unwrap();
    host.runtime.sessions.save("one",serde_json::from_value(json!({"id":"u1","parentId":null,"role":"user","createdAt":"2026-11-07T00:00:00Z","parts":[{"kind":"text","data":{"text":"while offline"}}],"extensions":{}})).unwrap(),None).unwrap();
    let (mut socket, _) = connect_async(request()).await.unwrap();
    socket
        .send(Message::Text(
            json!({"kind":"open","id":"second","sessionId":"one"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let frame: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(
        frame["snapshot"]["messages"][0]["parts"][0]["data"]["text"],
        "while offline"
    );
    let logout = host
        .client
        .post(format!("{}/api/v1/logout", host.base))
        .header("Origin", &host.base)
        .header("X-Minerva-CSRF", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    assert!(matches!(closed, Some(Ok(Message::Close(_))) | None));
    host.close().await;
}

#[tokio::test]
async fn auth_is_one_time_and_rpc_requires_session_origin_and_csrf() {
    let host = Host::start().await;
    let anonymous = reqwest::get(format!("{}/api/v1/bootstrap", host.base))
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let csrf = host.login().await;
    let replay = host
        .client
        .post(format!("{}/api/v1/auth/exchange", host.base))
        .header("Origin", &host.base)
        .json(&json!({"token":host.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 401);
    let wrong_origin = host
        .client
        .post(format!("{}/api/v1/rpc", host.base))
        .header("Origin", "https://evil.example")
        .header("X-Minerva-CSRF", &csrf)
        .json(&json!({"protocolVersion":1,"method":"session_list","params":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), 403);
    let missing_csrf = host.rpc("wrong", "session_list", json!({})).await;
    assert_eq!(missing_csrf["error"]["code"], "auth/csrf");
    host.close().await;
}

#[tokio::test]
async fn real_runtime_sessions_round_trip_and_invalid_methods_are_rejected() {
    let host = Host::start().await;
    let csrf = host.login().await;
    assert_eq!(
        host.rpc(&csrf, "session_save", json!({})).await["error"]["code"],
        "method/retired"
    );
    assert_eq!(
        host.rpc(&csrf, "session_run", json!({})).await["error"]["code"],
        "method/retired"
    );
    assert_eq!(
        host.rpc(&csrf, "session_create", json!({"id":"one"})).await["ok"],
        true
    );
    let session = host.rpc(&csrf, "session_get", json!({"id":"one"})).await;
    assert_eq!(session["value"]["id"], "one");
    assert_eq!(
        host.rpc(&csrf, "session_list", json!({})).await["value"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        host.rpc(&csrf, "session_create", json!({"id":"../escape"}))
            .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "session_create",
            json!({"id":"two","tenantId":"other"})
        )
        .await["error"]["code"],
        "request/invalid"
    );
    assert_eq!(
        host.rpc(&csrf, "unregistered", json!({})).await["error"]["code"],
        "method/not-found"
    );
    host.close().await;
}

#[tokio::test]
async fn static_fallback_never_turns_api_or_asset_misses_into_html() {
    let host = Host::start().await;
    std::fs::create_dir_all(host._dir.path().join("dist")).unwrap();
    std::fs::write(host._dir.path().join("dist/index.html"), "<h1>Minerva</h1>").unwrap();
    let page = reqwest::get(format!("{}/chat", host.base)).await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("Minerva"));
    for route in ["/api/unknown", "/plugins/missing.js", "/assets/missing.js"] {
        assert_eq!(
            reqwest::get(format!("{}{route}", host.base))
                .await
                .unwrap()
                .status(),
            404
        );
    }
    host.close().await;
}

#[test]
fn local_mode_rejects_external_origins() {
    assert!(LocalApiConfig::new("http://0.0.0.0:3080", ".".into()).is_err());
    assert!(LocalApiConfig::new("https://example.com", ".".into()).is_err());
}

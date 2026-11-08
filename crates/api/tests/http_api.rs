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

#[tokio::test]
async fn workspaces_are_system_owned_and_sessions_inherit_the_selected_directory() {
    let host = Host::start().await;
    let csrf = host.login().await;
    let initial = host.rpc(&csrf, "workspaces.list", json!({})).await;
    assert_eq!(initial["value"][0]["id"], "default");
    let project = host
        .rpc(&csrf, "workspaces.create", json!({"path":host._dir.path()}))
        .await;
    assert_eq!(project["ok"], true);
    let project = &project["value"];
    let created = host
        .rpc(
            &csrf,
            "session_create",
            json!({"id":"project-session","workspaceId":project["id"]}),
        )
        .await;
    assert_eq!(created["ok"], true);
    let settings = host
        .rpc(
            &csrf,
            "workspace.get",
            json!({"sessionId":"project-session"}),
        )
        .await;
    assert_eq!(settings["value"]["cwd"], project["path"]);
    let list = host.rpc(&csrf, "workspaces.list", json!({})).await;
    assert!(list["value"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == project["id"] && p["sessionIds"] == json!(["project-session"])));
    assert_eq!(
        host.rpc(
            &csrf,
            "session_create",
            json!({"id":"invalid-project","workspaceId":"unknown"})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(&csrf, "session_get", json!({"id":"invalid-project"}))
            .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "set_plugin_enabled",
            json!({"id":"workspaces","enabled":false})
        )
        .await["ok"],
        false
    );
    host.close().await;
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
async fn base_runtime_does_not_advertise_codex() {
    let host = Host::start().await;
    let csrf = host.login().await;
    let roster = host.rpc(&csrf, "list_plugins", json!({})).await;
    assert!(!roster["value"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "codex"));
    let bootstrap: Value = host
        .client
        .get(format!("{}/api/v1/bootstrap", host.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!bootstrap["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "codex.login"));
    for method in [
        "codex_models",
        "codex_status",
        "codex_quota",
        "codex_login",
        "codex_login_status",
    ] {
        let response = host.rpc(&csrf, method, json!({})).await;
        assert_eq!(response["ok"], false, "{method}");
    }
    host.close().await;
}

#[tokio::test]
async fn optional_plugins_switch_keeps_system_services_and_validates_arguments() {
    let host = Host::start().await;
    let csrf = host.login().await;
    let response = host
        .rpc(
            &csrf,
            "set_optional_plugins_enabled",
            json!({"enabled":false}),
        )
        .await;
    assert_eq!(response["ok"], true);
    for plugin in response["value"].as_array().unwrap() {
        assert_eq!(plugin["optionalEnabled"], false);
        assert_eq!(plugin["enabled"], plugin["canToggle"] == false);
    }
    assert_eq!(
        host.rpc(
            &csrf,
            "set_plugin_enabled",
            json!({"id":"plugin-manager","enabled":false})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "set_optional_plugins_enabled",
            json!({"enabled":"false"})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "set_optional_plugins_enabled",
            json!({"enabled":true,"system":true})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "set_optional_plugins_enabled",
            json!({"enabled":true})
        )
        .await["ok"],
        true
    );
    host.close().await;
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
    let policy = page.headers()["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("script-src 'self' blob:"));
    assert!(!policy.contains("unsafe-eval"));
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

#[tokio::test]
async fn harness_rpc_checks_session_ownership_and_strict_arguments() {
    let host = Host::start().await;
    let csrf = host.login().await;
    for id in ["images-one", "images-two"] {
        assert_eq!(
            host.rpc(&csrf, "session_create", json!({"id":id})).await["ok"],
            true
        );
    }
    for method in [
        "workspace.get",
        "tools.list",
        "interactions.list",
        "context.status",
    ] {
        assert_eq!(
            host.rpc(&csrf, method, json!({"sessionId":"unknown"}))
                .await["ok"],
            false,
            "{method}"
        );
        assert_eq!(
            host.rpc(
                &csrf,
                method,
                json!({"sessionId":"images-one","extra":true})
            )
            .await["ok"],
            false,
            "{method}"
        );
    }
    let upload = json!({"sessionId":"images-one","name":"pixel.png","mimeType":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="});
    let response = host.rpc(&csrf, "attachments.put", upload).await;
    assert_eq!(response["ok"], true, "{response}");
    let id = response["value"]["id"].as_str().unwrap();
    let own = host
        .rpc(
            &csrf,
            "attachments.get",
            json!({"sessionId":"images-one","id":id}),
        )
        .await;
    assert_eq!(own["ok"], true);
    assert_eq!(own["value"]["mimeType"], "image/png");
    for session in ["images-two", "missing"] {
        assert_eq!(
            host.rpc(
                &csrf,
                "attachments.get",
                json!({"sessionId":session,"id":id})
            )
            .await["ok"],
            false
        );
    }
    for id in ["../context.sqlite3", "", "bad/id"] {
        assert_eq!(
            host.rpc(
                &csrf,
                "attachments.get",
                json!({"sessionId":"images-one","id":id})
            )
            .await["ok"],
            false
        );
    }
    let invalid=host.rpc(&csrf,"attachments.put",json!({"sessionId":"images-one","name":"x.png","mimeType":"image/png","data":"dGV4dA=="})).await;
    assert_eq!(invalid["ok"], false);
    assert_eq!(
        host.rpc(&csrf, "mcp.list", json!({"extra":true})).await["ok"],
        false
    );
    assert_eq!(
        host.rpc(
            &csrf,
            "interactions.answer",
            json!({"sessionId":"images-two","id":"stale","decision":"allow"})
        )
        .await["value"],
        false
    );
    assert!(host
        .runtime
        .harness_call("engine.start", json!({}))
        .await
        .is_err());
    host.close().await;
}

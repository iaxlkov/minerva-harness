use minerva_api::{worker_router, WorkerApiConfig};
use minerva_runtime::{Runtime, RuntimeConfig};
use minerva_worker_auth::{body_digest, AssertionClaims, AssertionKey, HEADER};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn private_worker_rejects_unsigned_foreign_and_tampered_calls() {
    let temp = tempfile::tempdir().unwrap();
    let tenant = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: tenant.to_string(),
            workspace_id: workspace.to_string(),
            profile: "hosted-worker".into(),
            data_dir: temp.path().join("data"),
            config_dir: temp.path().join("config"),
        })
        .await
        .unwrap(),
    );
    let master = AssertionKey::new(vec![7; 32]).unwrap();
    let assignment = Uuid::new_v4();
    let worker = Uuid::new_v4();
    let key = master
        .derive_worker(assignment, 1, worker, tenant, workspace)
        .unwrap();
    let config = WorkerApiConfig {
        assignment_id: assignment,
        generation: 1,
        worker_id: worker,
        tenant_id: tenant,
        workspace_id: workspace,
        assertion_key: key.clone(),
    };
    let router = worker_router(runtime.clone(), config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v1/rpc", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let body =
        serde_json::to_vec(&json!({"protocolVersion":1,"method":"session_list","params":{}}))
            .unwrap();
    assert_eq!(
        client
            .post(&url)
            .body(body.clone())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut claims = AssertionClaims {
        version: 1,
        assignment_id: assignment,
        generation: 1,
        worker_id: worker,
        user_id: Uuid::new_v4(),
        tenant_id: tenant,
        workspace_id: workspace,
        action: "session_list".into(),
        http_method: "POST".into(),
        path: "/api/v1/rpc".into(),
        body_sha256: body_digest(&body),
        issued_at: now,
        expires_at: now + 30,
        request_id: Uuid::new_v4(),
    };
    let assertion = key.sign(&claims).unwrap();
    let response = client
        .post(&url)
        .header(HEADER, &assertion)
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["value"],
        json!([])
    );
    assert_eq!(
        client
            .post(&url)
            .header(HEADER, &assertion)
            .body(body.iter().copied().chain(*b" ").collect::<Vec<_>>())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    claims.workspace_id = Uuid::new_v4();
    assert_eq!(
        client
            .post(&url)
            .header(HEADER, key.sign(&claims).unwrap())
            .body(body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    task.abort();
    runtime.shutdown().await.unwrap();
}

use crate::{
    auth,
    directory::WorkerAssignment,
    http::{failure, success},
    registry::{method_action, validate_params},
    GatewayState,
};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use minerva_server_control::{
    audit::{AuditEvent, AuditKind},
    identity::Principal,
    policy::Action,
};
use minerva_worker_auth::{body_digest, AssertionClaims, HEADER};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RpcRequest {
    pub protocol_version: u32,
    pub method: String,
    pub params: Value,
}
pub(crate) fn parse_rpc(body: &[u8]) -> Result<(RpcRequest, Action), ()> {
    let request: RpcRequest = serde_json::from_slice(body).map_err(|_| ())?;
    if request.protocol_version != 1 {
        return Err(());
    }
    validate_params(&request.params).map_err(|_| ())?;
    let action = method_action(&request.method).ok_or(())?;
    Ok((request, action))
}
pub(crate) fn workspace_header(headers: &HeaderMap) -> Option<Uuid> {
    let mut values = headers.get_all("x-minerva-workspace").iter();
    let id = values.next()?.to_str().ok()?.parse().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(id)
}
pub(crate) fn assertion(
    state: &GatewayState,
    assignment: &WorkerAssignment,
    p: &Principal,
    action: &str,
    http_method: &str,
    path: &str,
    body: &[u8],
) -> Result<String, ()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs();
    let claims = AssertionClaims {
        version: 1,
        assignment_id: assignment.assignment_id,
        generation: assignment.generation,
        worker_id: assignment.worker_id,
        user_id: p.user_id(),
        tenant_id: assignment.resource.tenant_id,
        workspace_id: assignment.resource.workspace_id,
        action: action.into(),
        http_method: http_method.into(),
        path: path.into(),
        body_sha256: body_digest(body),
        issued_at: now,
        expires_at: now + 30,
        request_id: Uuid::new_v4(),
    };
    state
        .config
        .assertion_key
        .derive_worker(
            assignment.assignment_id,
            assignment.generation,
            assignment.worker_id,
            assignment.resource.tenant_id,
            assignment.resource.workspace_id,
        )
        .and_then(|key| key.sign(&claims))
        .map_err(|_| ())
}
pub async fn handle(State(state): State<GatewayState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let p = match auth::authenticate(&state, &parts.headers, true).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(workspace) = workspace_header(&parts.headers) else {
        return failure(StatusCode::NOT_FOUND, "resource/not-found");
    };
    let Ok(body) = to_bytes(body, 16 * 1024 * 1024).await else {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "request/too-large");
    };
    let (request, action) = match parse_rpc(&body) {
        Ok(v) => v,
        Err(_) => return failure(StatusCode::BAD_REQUEST, "request/invalid"),
    };
    let grant = match auth::workspace_grant(&state, &p, workspace, action).await {
        Ok(g) => g,
        Err(r) => {
            let _ = state
                .control
                .lock()
                .await
                .audit(&AuditEvent::new(
                    AuditKind::Denied,
                    Some(p.user_id()),
                    None,
                    serde_json::json!({"action":request.method}),
                ))
                .await;
            return r;
        }
    };
    let assignment = match state.directory.resolve(grant.resource()).await {
        Ok(a) => a,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable"),
    };
    if state.control.lock().await.revalidate(&grant).await.is_err() {
        return failure(StatusCode::NOT_FOUND, "resource/not-found");
    }
    let Ok(signed) = assertion(
        &state,
        &assignment,
        &p,
        &request.method,
        "POST",
        "/api/v1/rpc",
        &body,
    ) else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable");
    };
    let response = match state
        .http
        .post(assignment.rpc_url())
        .header(HEADER, signed)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable"),
    };
    let status = response.status();
    if !status.is_success() {
        return failure(
            if status == StatusCode::NOT_FOUND {
                StatusCode::NOT_FOUND
            } else if status.is_client_error()
                && status != StatusCode::UNAUTHORIZED
                && status != StatusCode::FORBIDDEN
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            if status == StatusCode::NOT_FOUND {
                "resource/not-found"
            } else {
                "worker/request-failed"
            },
        );
    }
    let response_limit = if request.method == "attachments.get" {
        16 * 1024 * 1024
    } else {
        4 * 1024 * 1024
    };
    let bytes = match crate::bounded_body(response, response_limit).await {
        Ok(b) => b,
        Err(_) => return failure(StatusCode::BAD_GATEWAY, "worker/invalid-response"),
    };
    let data: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return failure(StatusCode::BAD_GATEWAY, "worker/invalid-response"),
    };
    if data.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return failure(StatusCode::BAD_GATEWAY, "worker/invalid-response");
    }
    let Some(value) = data.get("value") else {
        return failure(StatusCode::BAD_GATEWAY, "worker/invalid-response");
    };
    if !matches!(state.directory.resolve(grant.resource()).await,Ok(current) if current==assignment)
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable");
    }
    if matches!(
        request.method.as_str(),
        "set_plugin_enabled" | "set_optional_plugins_enabled"
    ) && state
        .control
        .lock()
        .await
        .audit(&AuditEvent::new(
            AuditKind::PluginActivated,
            Some(p.user_id()),
            Some(workspace),
            serde_json::json!({"action":"plugins.activate"}),
        ))
        .await
        .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "control/unavailable");
    }
    if state.control.lock().await.revalidate(&grant).await.is_err() {
        return failure(StatusCode::NOT_FOUND, "resource/not-found");
    }
    success(value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rpc_rejects_unknown_envelope_fields_and_unsupported_versions() {
        assert!(parse_rpc(
            br#"{"protocolVersion":1,"method":"session_list","params":{},"workspaceId":"forged"}"#
        )
        .is_err());
        assert!(
            parse_rpc(br#"{"protocolVersion":2,"method":"session_list","params":{}}"#).is_err()
        );
        assert!(parse_rpc(br#"{"protocolVersion":1,"method":"unknown","params":{}}"#).is_err());
        assert!(parse_rpc(br#"{"protocolVersion":1,"method":"session_list","params":{}}"#).is_ok());
    }
}

use crate::{
    auth,
    directory::WorkerAssignment,
    http::{failure, origin_allowed},
    rpc, GatewayState,
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use minerva_server_control::{
    identity::Principal,
    membership::RevocationScope,
    policy::{Action, AuthorizedWorkspace},
};
use minerva_worker_auth::HEADER;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message as WorkerMessage},
    MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ClientFrame {
    Open {
        id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Close {
        id: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum WorkerFrame {
    Baseline {
        id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        snapshot: Value,
    },
    Event {
        id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        seq: u64,
        #[serde(rename = "eventType")]
        event_type: String,
        data: Value,
    },
    Resync {
        id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Error {
        id: String,
        #[serde(rename = "sessionId", default)]
        session_id: Option<String>,
        code: String,
    },
}
struct Subscription {
    session_id: String,
    cursor: Option<u64>,
    awaiting_baseline: bool,
}
#[derive(Default)]
struct Scopes {
    sessions: HashMap<String, Subscription>,
}
const MAX_SAFE_CURSOR: u64 = 9_007_199_254_740_991;
fn nullable_string(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Null | Value::String(_)))
}
fn parts(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|parts| {
        parts
            .iter()
            .all(|p| p.get("kind").and_then(Value::as_str).is_some() && p.get("data").is_some())
    })
}
fn session_info(value: &Value, session: &str) -> bool {
    value.get("id").and_then(Value::as_str) == Some(session)
        && value.get("title").and_then(Value::as_str).is_some()
        && value.get("updatedAt").and_then(Value::as_i64).is_some()
        && nullable_string(value.get("model"))
}
fn message(value: &Value) -> bool {
    value.get("id").and_then(Value::as_str).is_some()
        && nullable_string(value.get("parentId"))
        && value.get("role").and_then(Value::as_str).is_some()
        && value.get("createdAt").and_then(Value::as_str).is_some()
        && parts(value.get("parts"))
        && value.get("extensions").is_some_and(Value::is_object)
}
fn run(value: &Value, session: &str) -> bool {
    value.get("sessionId").and_then(Value::as_str) == Some(session)
        && value.get("runId").and_then(Value::as_str).is_some()
        && matches!(
            value.get("status").and_then(Value::as_str),
            Some("running" | "completed" | "failed" | "cancelled" | "interrupted")
        )
        && parts(value.get("parts"))
        && nullable_string(value.get("error"))
}
impl Scopes {
    fn accept_client(&mut self, text: &str) -> Result<(), ()> {
        match serde_json::from_str::<ClientFrame>(text).map_err(|_| ())? {
            ClientFrame::Open { id, session_id } => {
                if self.sessions.len() >= 32
                    || id.is_empty()
                    || id.len() > 128
                    || session_id.is_empty()
                    || session_id.len() > 256
                    || self.sessions.contains_key(&id)
                {
                    return Err(());
                }
                self.sessions.insert(
                    id,
                    Subscription {
                        session_id,
                        cursor: None,
                        awaiting_baseline: true,
                    },
                );
            }
            ClientFrame::Close { id } => {
                self.sessions.remove(&id);
            }
        }
        Ok(())
    }
    fn accept_worker(&mut self, text: &str) -> Result<(), ()> {
        match serde_json::from_str::<WorkerFrame>(text).map_err(|_| ())? {
            WorkerFrame::Baseline {
                id,
                session_id,
                snapshot,
            } => {
                let subscription = self.sessions.get_mut(&id).ok_or(())?;
                if subscription.session_id != session_id {
                    return Err(());
                }
                let cursor = snapshot
                    .get("cursor")
                    .and_then(Value::as_u64)
                    .filter(|c| *c <= MAX_SAFE_CURSOR)
                    .ok_or(())?;
                if subscription.cursor.is_some_and(|old| cursor < old)
                    || !snapshot
                        .get("session")
                        .is_some_and(|v| session_info(v, &session_id))
                    || !snapshot
                        .get("messages")
                        .and_then(Value::as_array)
                        .is_some_and(|messages| messages.iter().all(message))
                    || !snapshot
                        .get("activeRun")
                        .is_some_and(|v| v.is_null() || run(v, &session_id))
                {
                    return Err(());
                }
                subscription.cursor = Some(cursor);
                subscription.awaiting_baseline = false;
            }
            WorkerFrame::Event {
                id,
                session_id,
                seq,
                event_type,
                data,
            } => {
                let subscription = self.sessions.get_mut(&id).ok_or(())?;
                if subscription.session_id != session_id
                    || subscription.awaiting_baseline
                    || seq > MAX_SAFE_CURSOR
                    || subscription.cursor.and_then(|c| c.checked_add(1)) != Some(seq)
                {
                    return Err(());
                }
                let valid = match event_type.as_str() {
                    "session.updated" => session_info(&data, &session_id),
                    "message.upsert" => message(&data),
                    "run.updated" => run(&data, &session_id),
                    _ => false,
                };
                if !valid {
                    return Err(());
                }
                subscription.cursor = Some(seq);
            }
            WorkerFrame::Resync { id, session_id } => {
                let subscription = self.sessions.get_mut(&id).ok_or(())?;
                if subscription.session_id != session_id
                    || subscription.cursor.is_none()
                    || subscription.awaiting_baseline
                {
                    return Err(());
                }
                subscription.awaiting_baseline = true;
            }
            WorkerFrame::Error {
                id,
                session_id,
                code,
            } => {
                let subscription = self.sessions.get(&id).ok_or(())?;
                if session_id
                    .as_ref()
                    .is_some_and(|s| *s != subscription.session_id)
                    || code != "resource/not-found"
                {
                    return Err(());
                }
            }
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamQuery {
    workspace_id: Uuid,
}
pub async fn upgrade(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, &state.config.public_origin, true) {
        return failure(StatusCode::FORBIDDEN, "auth/origin");
    }
    let p = match auth::authenticate(&state, &headers, false).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let grant = match auth::workspace_grant(&state, &p, query.workspace_id, Action::Read).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    let assignment = match state.directory.resolve(grant.resource()).await {
        Ok(a) => a,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable"),
    };
    let Ok(signed) = rpc::assertion(
        &state,
        &assignment,
        &p,
        "streams.connect",
        "GET",
        "/api/v1/streams",
        b"",
    ) else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable");
    };
    let Ok(mut request) = assignment.streams_url().into_client_request() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable");
    };
    let Ok(signed) = signed.parse() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable");
    };
    request.headers_mut().insert(HEADER, signed);
    let upstream = match tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async_with_config(
            request,
            Some(
                tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(4 * 1024 * 1024))
                    .max_frame_size(Some(1024 * 1024)),
            ),
            false,
        ),
    )
    .await
    {
        Ok(Ok((stream, _))) => stream,
        _ => return failure(StatusCode::SERVICE_UNAVAILABLE, "worker/unavailable"),
    };
    let revocations = state.control.lock().await.subscribe_revocations();
    ws.max_message_size(65536)
        .on_upgrade(move |socket| serve(socket, state, p, grant, assignment, upstream, revocations))
}
async fn valid(
    state: &GatewayState,
    grant: &AuthorizedWorkspace,
    assignment: &WorkerAssignment,
) -> bool {
    if !matches!(state.directory.resolve(grant.resource()).await,Ok(current) if &current==assignment)
    {
        return false;
    }
    state.control.lock().await.revalidate(grant).await.is_ok()
}
async fn serve(
    socket: WebSocket,
    state: GatewayState,
    _p: Principal,
    grant: AuthorizedWorkspace,
    assignment: WorkerAssignment,
    upstream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    mut revocations: tokio::sync::broadcast::Receiver<RevocationScope>,
) {
    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut worker_tx, mut worker_rx) = upstream.split();
    let mut scopes = Scopes::default();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            notification=revocations.recv()=>{match notification{Ok(scope) if !scope.affects(&grant)=>{},_=>break}},
            _=tick.tick()=>{if !valid(&state,&grant,&assignment).await{break;}},
            incoming=browser_rx.next()=>{
                let Some(Ok(message))=incoming else{break};
                match message{
                    Message::Text(text)=>{if scopes.accept_client(&text).is_err() || !valid(&state,&grant,&assignment).await{break;}
                        if !matches!(tokio::time::timeout(Duration::from_secs(5),worker_tx.send(WorkerMessage::Text(text.to_string().into()))).await,Ok(Ok(()))){break;}},
                    Message::Ping(bytes)=>{if !matches!(tokio::time::timeout(Duration::from_secs(5),browser_tx.send(Message::Pong(bytes))).await,Ok(Ok(()))){break;}},Message::Close(_)=>break,_=>break,
                }
            },
            incoming=worker_rx.next()=>{
                let Some(Ok(message))=incoming else{break};
                match message{
                    WorkerMessage::Text(text)=>{
                        if text.len()>4*1024*1024 || scopes.accept_worker(&text).is_err() || !valid(&state,&grant,&assignment).await{break;}
                        let mut output=text.to_string();
                        if let Ok(value)=serde_json::from_str::<Value>(&text){if value.get("kind").and_then(|v|v.as_str())==Some("error"){let session=value.get("id").and_then(Value::as_str).and_then(|id|scopes.sessions.get(id)).map(|s|&s.session_id);output=serde_json::json!({"kind":"error","id":value.get("id"),"sessionId":session,"code":"resource/not-found"}).to_string();}}
                        if !matches!(tokio::time::timeout(Duration::from_secs(5),browser_tx.send(Message::Text(output.into()))).await,Ok(Ok(()))){break;}
                    },
                    WorkerMessage::Ping(bytes)=>{if !matches!(tokio::time::timeout(Duration::from_secs(5),worker_tx.send(WorkerMessage::Pong(bytes))).await,Ok(Ok(()))){break;}},WorkerMessage::Close(_)=>break,_=>{},
                }
            }
        }
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        browser_tx.send(Message::Close(None)),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        worker_tx.send(WorkerMessage::Close(None)),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_scopes_are_explicit_and_close_blocks_stale_deliveries() {
        let mut scopes = Scopes::default();
        assert!(scopes
            .accept_client(
                r#"{"kind":"open","id":"one","sessionId":"alice-session","workspaceId":"forged"}"#
            )
            .is_err());
        assert!(scopes
            .accept_client(r#"{"kind":"open","id":"one","sessionId":"alice-session"}"#)
            .is_ok());
        assert!(scopes
            .accept_worker(
                r#"{"kind":"baseline","id":"one","sessionId":"bob-session","snapshot":{}}"#
            )
            .is_err());
        assert!(scopes
            .accept_worker(
                r#"{"kind":"baseline","id":"one","sessionId":"alice-session","snapshot":{"session":{"id":"alice-session","title":"Chat","updatedAt":0,"model":null},"messages":[],"cursor":0,"activeRun":null}}"#
            )
            .is_ok());
        assert!(scopes
            .accept_worker(
                r#"{"kind":"baseline","id":"unopened","sessionId":"alice-session","snapshot":{}}"#
            )
            .is_err());
        assert!(scopes
            .accept_client(r#"{"kind":"close","id":"one"}"#)
            .is_ok());
        assert!(scopes
            .accept_worker(
                r#"{"kind":"baseline","id":"one","sessionId":"alice-session","snapshot":{"session":{"id":"alice-session","title":"Chat","updatedAt":0,"model":null},"messages":[],"cursor":0,"activeRun":null}}"#
            )
            .is_err());
    }
}
#[cfg(test)]
mod event_protocol_tests {
    use super::*;
    use serde_json::json;
    fn baseline(cursor: u64) -> String {
        json!({"kind":"baseline","id":"one","sessionId":"alice-session","snapshot":{"session":{"id":"alice-session","title":"Chat","updatedAt":0,"model":null},"messages":[],"cursor":cursor,"activeRun":null}}).to_string()
    }
    fn opened() -> Scopes {
        let mut scopes = Scopes::default();
        scopes
            .accept_client(r#"{"kind":"open","id":"one","sessionId":"alice-session"}"#)
            .unwrap();
        scopes.accept_worker(&baseline(10)).unwrap();
        scopes
    }
    fn event(seq: u64, event_type: &str, data: Value) -> String {
        json!({"kind":"event","id":"one","sessionId":"alice-session","seq":seq,"eventType":event_type,"data":data}).to_string()
    }
    #[test]
    fn known_events_and_resync_baseline_follow_real_worker_protocol() {
        let mut scopes = opened();
        assert!(scopes
            .accept_worker(&event(
                11,
                "session.updated",
                json!({"id":"alice-session","title":"Renamed","updatedAt":1,"model":null})
            ))
            .is_ok());
        assert!(scopes.accept_worker(&event(12,"message.upsert",json!({"id":"message","parentId":null,"role":"user","createdAt":"2026-11-07T00:00:00Z","parts":[],"extensions":{}}))).is_ok());
        assert!(scopes.accept_worker(&event(13,"run.updated",json!({"sessionId":"alice-session","runId":"run","status":"running","parts":[],"error":null}))).is_ok());
        assert!(scopes
            .accept_worker(r#"{"kind":"resync","id":"one","sessionId":"alice-session"}"#)
            .is_ok());
        assert!(scopes
            .accept_worker(&event(
                14,
                "session.updated",
                json!({"id":"alice-session","title":"Old","updatedAt":1,"model":null})
            ))
            .is_err());
        assert!(scopes.accept_worker(&baseline(300)).is_ok());
        assert!(scopes
            .accept_worker(&event(
                301,
                "session.updated",
                json!({"id":"alice-session","title":"Current","updatedAt":2,"model":null})
            ))
            .is_ok());
    }
    #[test]
    fn foreign_unsolicited_and_invalid_cursor_or_event_type_never_deliver() {
        let mut scopes = opened();
        let data = json!({"id":"alice-session","title":"Chat","updatedAt":1,"model":null});
        let real: Value =
            serde_json::from_str(&event(11, "session.updated", data.clone())).unwrap();
        for (key, value) in [
            ("id", json!("unopened")),
            ("sessionId", json!("bob-session")),
            ("seq", json!(10)),
            ("seq", json!(12)),
            ("seq", json!(-1)),
            ("seq", json!(9007199254740992u64)),
            ("eventType", json!("unknown")),
            ("data", json!("invalid")),
        ] {
            let mut frame = real.clone();
            frame[key] = value;
            assert!(scopes.accept_worker(&frame.to_string()).is_err(), "{frame}");
        }
        assert!(scopes.accept_worker(&event(11,"run.updated",json!({"sessionId":"bob-session","runId":"run","status":"running","parts":[],"error":null}))).is_err());
        assert!(scopes
            .accept_worker(r#"{"kind":"resync","id":"one","sessionId":"bob-session"}"#)
            .is_err());
        assert!(scopes
            .accept_worker(r#"{"kind":"resync","id":"unopened","sessionId":"alice-session"}"#)
            .is_err());
        assert!(scopes
            .accept_worker(&event(11, "session.updated", data))
            .is_ok());
        scopes
            .accept_client(r#"{"kind":"close","id":"one"}"#)
            .unwrap();
        assert!(scopes
            .accept_worker(&event(
                12,
                "session.updated",
                json!({"id":"alice-session","title":"Stale","updatedAt":2,"model":null})
            ))
            .is_err());
    }
}
#[cfg(test)]
mod baseline_protocol_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn malformed_baselines_and_events_without_a_baseline_are_rejected() {
        let mut scopes = Scopes::default();
        scopes
            .accept_client(r#"{"kind":"open","id":"one","sessionId":"alice-session"}"#)
            .unwrap();
        assert!(scopes
            .accept_worker(r#"{"kind":"resync","id":"one","sessionId":"alice-session"}"#)
            .is_err());
        assert!(scopes.accept_worker(r#"{"kind":"event","id":"one","sessionId":"alice-session","seq":1,"eventType":"message.upsert","data":{"id":"m","parentId":null,"role":"user","createdAt":"now","parts":[],"extensions":{}}}"#).is_err());
        let real = json!({"kind":"baseline","id":"one","sessionId":"alice-session","snapshot":{"session":{"id":"alice-session","title":"Chat","updatedAt":0,"model":null},"messages":[],"cursor":10,"activeRun":null}});
        for (key, value) in [
            ("cursor", json!(-1)),
            ("cursor", json!(9007199254740992u64)),
            ("messages", json!({})),
            (
                "session",
                json!({"id":"bob-session","title":"Chat","updatedAt":0,"model":null}),
            ),
            (
                "activeRun",
                json!({"sessionId":"bob-session","runId":"r","status":"running","parts":[],"error":null}),
            ),
        ] {
            let mut frame = real.clone();
            frame["snapshot"][key] = value;
            assert!(scopes.accept_worker(&frame.to_string()).is_err());
        }
        assert!(scopes.accept_worker(&real.to_string()).is_ok());
        let mut old = real;
        old["snapshot"]["cursor"] = json!(9);
        assert!(scopes.accept_worker(&old.to_string()).is_err());
    }
}

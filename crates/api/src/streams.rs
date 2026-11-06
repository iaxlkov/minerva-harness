use super::{auth, failure, rpc, ApiState};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, time::Duration};

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
pub(crate) async fn upgrade(
    State(state): State<ApiState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(r) = auth::authorize(&state, &headers, false) {
        return *r;
    }
    if !headers.contains_key("origin") {
        return failure(
            StatusCode::FORBIDDEN,
            "auth/origin",
            "WebSocket requires an Origin",
        );
    }
    let runtime = state.runtime.clone();
    ws.max_message_size(65536).on_upgrade(move |socket| {
        serve_workspace(socket, runtime, move || {
            auth::authorize(&state, &headers, false).is_ok()
        })
    })
}
pub(crate) async fn serve_workspace(
    socket: WebSocket,
    runtime: std::sync::Arc<minerva_runtime::Runtime>,
    authorized: impl Fn() -> bool + Send + 'static,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut subscriptions: HashMap<String, (String, i64)> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            incoming=receiver.next()=>{
                let Some(Ok(message))=incoming else{break};
                match message {
                    Message::Text(text)=>{
                        let frame=serde_json::from_str::<ClientFrame>(&text);
                        match frame {
                            Ok(ClientFrame::Open{id,session_id}) if subscriptions.len()<32 && id.len()<=128=>{
                                match rpc::session_snapshot(&runtime,&session_id){
                                    Ok(snapshot)=>{
                                        if send(&mut sender,Message::Text(json!({"kind":"baseline","id":id,"sessionId":session_id,"snapshot":snapshot}).to_string().into())).await.is_err(){break;}
                                        let cursor=snapshot["cursor"].as_i64().unwrap_or(0);
                                        subscriptions.insert(id,(session_id,cursor));
                                    },
                                    Err(_)=>{if send(&mut sender,Message::Text(json!({"kind":"error","id":id,"sessionId":session_id,"code":"resource/not-found"}).to_string().into())).await.is_err(){break;}},
                                }
                            },
                            Ok(ClientFrame::Close{id})=>{subscriptions.remove(&id);},
                            _=>{let _=send(&mut sender,Message::Close(None)).await;break;},
                        }
                    },
                    Message::Ping(bytes)=>{if send(&mut sender,Message::Pong(bytes)).await.is_err(){break;}},
                    Message::Close(_)=>break,
                    _=>{},
                }
            },
            _=tick.tick()=>{
                if runtime.is_stopped().await {let _=send(&mut sender,Message::Close(None)).await;break;}
                if !authorized(){let _=send(&mut sender,Message::Close(Some(axum::extract::ws::CloseFrame{code:1008,reason:"authorization expired".into()}))).await;break;}
                for (id,(session_id,cursor)) in &mut subscriptions {
                    let Ok(events)=runtime.sessions.events_after(session_id,*cursor,256) else{return};
                    if events.len()==256 {
                        if send(&mut sender,Message::Text(json!({"kind":"resync","id":id,"sessionId":session_id}).to_string().into())).await.is_err(){return;}
                        let Ok(snapshot)=rpc::session_snapshot(&runtime,session_id) else{return};
                        *cursor=snapshot["cursor"].as_i64().unwrap_or(0);
                        let frame=Message::Text(json!({"kind":"baseline","id":id,"sessionId":session_id,"snapshot":snapshot}).to_string().into());
                        if send(&mut sender,frame).await.is_err(){return;}
                    } else {
                        for event in events {
                            let frame=Message::Text(json!({"kind":"event","id":id,"sessionId":session_id,"seq":event.seq,"eventType":event.kind,"data":event.data}).to_string().into());
                            if send(&mut sender,frame).await.is_err(){return;}
                            *cursor=event.seq;
                        }
                    }
                }
            }
        }
    }
}

async fn send(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: Message,
) -> Result<(), ()> {
    tokio::time::timeout(Duration::from_secs(5), sender.send(message))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

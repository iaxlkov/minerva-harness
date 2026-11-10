use super::*;
use std::sync::atomic::AtomicU64;
use tokio::sync::oneshot;
struct Pending {
    view: PendingInteraction,
    reply: oneshot::Sender<String>,
    cancellation: Cancellation,
    expires: tokio::time::Instant,
}
#[derive(Default)]
pub struct InteractionBroker {
    pending: Mutex<BTreeMap<String, Pending>>,
    next: AtomicU64,
}
impl cordis::Service for InteractionBroker {
    const NAME: &'static str = "studio.interactions";
}
impl InteractionBroker {
    pub fn list(&self, session_id: &str) -> Vec<PendingInteraction> {
        self.pending
            .lock()
            .expect("interaction broker poisoned")
            .values()
            .filter(|p| {
                p.view.session_id == session_id
                    && !p.cancellation.is_cancelled()
                    && p.expires > tokio::time::Instant::now()
            })
            .map(|p| p.view.clone())
            .collect()
    }
    pub fn answer(&self, answer: InteractionAnswer) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let Some(item) = pending.get(&answer.id) else {
            return false;
        };
        if item.view.session_id != answer.session_id
            || item.cancellation.is_cancelled()
            || item.expires <= tokio::time::Instant::now()
        {
            return false;
        }
        let valid = if item.view.kind == "approval" {
            matches!(answer.decision.as_str(), "allow" | "deny") && answer.answer.is_none()
        } else {
            answer.decision == "answer"
                && answer.answer.as_ref().is_some_and(|s| {
                    !s.trim().is_empty()
                        && s.len() <= 65536
                        && (item.view.options.is_empty() || item.view.options.contains(s))
                })
        };
        if !valid {
            return false;
        }
        let item = pending.remove(&answer.id).expect("checked pending item");
        item.reply
            .send(answer.answer.unwrap_or(answer.decision))
            .is_ok()
    }
    pub fn cancel_run(&self, session: &str, run: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, p| p.view.session_id != session || p.view.run_id != run);
        }
    }
    pub fn cancel_call(&self, ctx: &ToolContext) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, p| {
                p.view.session_id != ctx.session_id
                    || p.view.run_id != ctx.run_id
                    || p.view.call_id != ctx.call_id
            });
        }
    }
    pub async fn request(
        &self,
        ctx: &ToolContext,
        kind: &str,
        title: &str,
        prompt: &str,
        options: Vec<String>,
    ) -> Result<String, String> {
        if ctx.session_id.is_empty()
            || ctx.run_id.is_empty()
            || ctx.call_id.is_empty()
            || !matches!(kind, "approval" | "question")
            || prompt.is_empty()
            || prompt.len() > 65536
            || options.len() > 32
        {
            return Err("invalid interaction request".into());
        }
        if ctx.cancellation.is_cancelled() {
            return Err("interaction cancelled".into());
        }
        let id = format!(
            "interaction-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos(),
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let view = PendingInteraction {
            id: id.clone(),
            session_id: ctx.session_id.clone(),
            run_id: ctx.run_id.clone(),
            call_id: ctx.call_id.clone(),
            kind: kind.into(),
            title: title.into(),
            prompt: prompt.into(),
            options,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis() as f64,
        };
        let (reply, rx) = oneshot::channel();
        self.pending.lock().map_err(|e| e.to_string())?.insert(
            id.clone(),
            Pending {
                view,
                reply,
                cancellation: ctx.cancellation.clone(),
                expires: tokio::time::Instant::now() + Duration::from_secs(600),
            },
        );
        struct Cleanup<'a>(&'a InteractionBroker, String);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                if let Ok(mut pending) = self.0.pending.lock() {
                    pending.remove(&self.1);
                }
            }
        }
        let _cleanup = Cleanup(self, id);
        tokio::select! {result=rx=>result.map_err(|_|"interaction expired or cancelled".into()),_=ctx.cancellation.cancelled()=>Err("interaction cancelled".into()),_=tokio::time::sleep(Duration::from_secs(600))=>Err("interaction expired".into())}
    }
}

//! In-process ownership and cancellation of one turn per session.
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::watch;

#[derive(Default)]
pub struct Runs(Mutex<HashMap<String, (String, watch::Sender<bool>)>>);
pub struct RunGuard<'a> {
    runs: &'a Runs,
    session: String,
    pub cancel: watch::Receiver<bool>,
}
impl Runs {
    pub fn acquire(&self, session: &str, id: &str) -> Result<RunGuard<'_>, String> {
        let mut runs = self.0.lock().map_err(|e| e.to_string())?;
        if runs.contains_key(session) {
            return Err("В этом диалоге уже выполняется запрос.".into());
        }
        let (tx, rx) = watch::channel(false);
        runs.insert(session.into(), (id.into(), tx));
        Ok(RunGuard {
            runs: self,
            session: session.into(),
            cancel: rx,
        })
    }
    pub fn cancel(&self, session: &str, id: &str) -> Result<bool, String> {
        let runs = self.0.lock().map_err(|e| e.to_string())?;
        if let Some((active, sender)) = runs.get(session) {
            if active == id {
                sender.send_replace(true);
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runs.0.lock() {
            runs.remove(&self.session);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn excludes_duplicates_cancels_only_matching_run_and_releases() {
        let runs = Runs::default();
        let mut first = runs.acquire("a", "one").unwrap();
        assert!(runs.acquire("a", "two").is_err());
        let _other = runs.acquire("b", "one").unwrap();
        assert!(!runs.cancel("a", "old").unwrap());
        assert!(runs.cancel("a", "one").unwrap());
        first.cancel.changed().await.unwrap();
        assert!(*first.cancel.borrow());
        drop(first);
        assert!(runs.acquire("a", "two").is_ok());
    }
}

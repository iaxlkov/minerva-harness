//! Host-owned optional plugins share the manager lifecycle without a dependency on runtime.
use crate::plugins::PluginInfo;
use serde_json::Value;
use std::{future::Future, pin::Pin};
pub type OptionalFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;
pub trait OptionalPlugins: Send + Sync {
    fn owns(&self, id: &str) -> bool;
    fn list(&self) -> Vec<PluginInfo>;
    fn set_enabled<'a>(&'a self, id: &'a str, enabled: bool) -> OptionalFuture<'a, ()>;
    fn set_optional_enabled(&self, enabled: bool) -> OptionalFuture<'_, ()>;
    fn call<'a>(&'a self, id: &'a str, method: &'a str, params: Value)
        -> OptionalFuture<'a, Value>;
    fn shutdown(&self) -> OptionalFuture<'_, ()>;
}

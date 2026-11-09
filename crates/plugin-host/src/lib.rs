//! Local trusted executable host. This process boundary is NOT a security sandbox.
use minerva_plugin_sdk::Manifest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
pub mod config;
mod graph;
mod package;
mod process;
mod storage;
pub use package::{install_local, Package};
pub use process::Process;
use storage::*;

const LIMIT: usize = 1024 * 1024;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub struct PluginStatus {
    pub id: String,
    pub manifest: Option<Manifest>,
    pub enabled: bool,
    pub requested_enabled: bool,
    pub error: Option<String>,
    pub generation: u64,
}
#[derive(Default)]
struct State {
    package: Option<Package>,
    enabled: bool,
    requested_enabled: bool,
    generation: u64,
    error: Option<String>,
    catalog_error: Option<String>,
}
struct Entry {
    state: Mutex<State>,
    process: tokio::sync::Mutex<Option<Process>>,
}
impl Entry {
    fn reap(&self) {
        if let Ok(mut process) = self.process.try_lock() {
            if process
                .as_mut()
                .is_some_and(|p| !matches!(p.child.try_wait(), Ok(None)))
            {
                process.take();
                let mut state = self.state.lock().unwrap();
                state.enabled = false;
                state.error = Some("plugin process exited".into());
            }
        }
    }
}
// Taking ownership ensures cancellation kills the process instead of reusing an unread response.
struct Flight<'a> {
    process: Option<Process>,
    state: &'a Mutex<State>,
}
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        if self.process.is_some() {
            let mut state = self.state.lock().unwrap();
            state.enabled = false;
            state.error = Some("plugin operation cancelled".into());
        }
    }
}
// Cancellation must also dispose idle processes fenced before an awaited shutdown.
struct CleanupDisabled<'a>(&'a Host);
impl Drop for CleanupDisabled<'_> {
    fn drop(&mut self) {
        let mut stopped: Vec<String> = self
            .0
            .entries
            .iter()
            .filter(|(_, entry)| !entry.state.lock().unwrap().enabled)
            .map(|(id, _)| id.clone())
            .collect();
        self.0.dependents_first(&mut stopped);
        for id in stopped {
            if let Ok(mut process) = self.0.entries[&id].process.try_lock() {
                process.take();
            }
        }
    }
}
pub struct Host {
    config: PathBuf,
    entries: BTreeMap<String, Entry>,
    local: bool,
    optional: std::sync::atomic::AtomicBool,
    example: std::sync::atomic::AtomicBool,
    lifecycle: tokio::sync::Mutex<()>,
}
impl Host {
    /// Hosted mode is deliberately disabled until a sandbox execution provider is integrated.
    pub fn open(config: PathBuf, local: bool) -> Result<Self, String> {
        let composition = config::ConfigStore::open(&config)?.snapshot()?;
        let lock = read_lock(&config)?;
        let mut entries = BTreeMap::new();
        let dir = config.join("plugins");
        if dir.exists() {
            for entry in fs::read_dir(dir).map_err(err)? {
                let p = entry.map_err(err)?.path();
                let id = p.file_name().unwrap().to_string_lossy().into_owned();
                if id.starts_with('.') {
                    continue;
                }
                let result = Self::load_package(&p, &id);
                let (package, error) = match result {
                    Ok(package) => (Some(package), None),
                    Err(error) => (None, Some(error)),
                };
                entries.insert(
                    id,
                    Entry {
                        state: Mutex::new(State {
                            package,
                            error,
                            ..State::default()
                        }),
                        process: tokio::sync::Mutex::new(None),
                    },
                );
            }
        }
        for id in lock
            .plugins
            .keys()
            .chain(composition.plugins.keys().filter(|id| {
                id.as_str() != "example"
                    && !minerva_plugin_sdk::SYSTEM_PLUGIN_IDS.contains(&id.as_str())
            }))
        {
            entries.entry(id.clone()).or_insert_with(|| Entry {
                state: Mutex::new(State {
                    error: Some("plugin package missing".into()),
                    ..State::default()
                }),
                process: tokio::sync::Mutex::new(None),
            });
        }
        for (id, entry) in &entries {
            entry.state.lock().unwrap().requested_enabled =
                composition.plugins.get(id).is_some_and(|p| p.enabled);
        }
        Ok(Self {
            optional: std::sync::atomic::AtomicBool::new(composition.optional_plugins_enabled),
            example: std::sync::atomic::AtomicBool::new(
                composition.plugins.get("example").is_none_or(|p| p.enabled),
            ),
            lifecycle: tokio::sync::Mutex::new(()),
            config,
            entries,
            local,
        })
    }
    fn load_package(path: &Path, id: &str) -> Result<Package, String> {
        // Lock-file IDs are untrusted paths, including entries for missing packages.
        minerva_plugin_sdk::relative_entry(id)?;
        if id.contains(['/', '\\']) {
            return Err("invalid plugin directory".into());
        }
        let package = Package::load(path)?;
        if package.manifest.id != id {
            return Err("package directory/id mismatch".into());
        }
        Ok(package)
    }
    pub fn roster(&self) -> Vec<PluginStatus> {
        self.reap_dependencies();
        self.entries
            .iter()
            .map(|(id, entry)| {
                entry.reap();
                let state = entry.state.lock().unwrap();
                PluginStatus {
                    id: id.clone(),
                    manifest: state.package.as_ref().map(|p| p.manifest.clone()),
                    enabled: state.enabled,
                    requested_enabled: state.requested_enabled,
                    error: state.error.clone().or_else(|| state.catalog_error.clone()),
                    generation: state.generation,
                }
            })
            .collect()
    }
    pub fn set_catalog_error(&self, id: &str, generation: u64, error: Option<String>) {
        if let Some(entry) = self.entries.get(id) {
            let mut state = entry.state.lock().unwrap();
            if state.generation == generation {
                state.catalog_error = error;
            }
        }
    }
    pub async fn restore(&self) -> Result<(), String> {
        let _guard = self.lifecycle.lock().await;
        self.reconcile().await?;
        Ok(())
    }
    pub fn optional_enabled(&self) -> bool {
        self.optional.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn example_enabled(&self) -> bool {
        self.example.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub async fn set_example_enabled(&self, enabled: bool) -> Result<(), String> {
        let guard = WriterGuard::acquire_async(self.config.clone()).await?;
        config::ConfigStore::at(&self.config).update_locked(
            |c| {
                c.plugins
                    .insert("example".into(), config::PluginConfig { enabled });
                Ok(())
            },
            &guard,
        )?;
        self.example
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    pub async fn set_optional_enabled(&self, enabled: bool) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let guard = WriterGuard::acquire_async(self.config.clone()).await?;
            config::ConfigStore::at(&self.config).update_locked(
                |c| {
                    c.optional_plugins_enabled = enabled;
                    Ok(())
                },
                &guard,
            )?;
        }
        self.optional
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        self.reconcile().await?;
        Ok(())
    }
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock().await;
        let entry = self.entries.get(id).ok_or("unknown external plugin")?;
        {
            let guard = WriterGuard::acquire_async(self.config.clone()).await?;
            let mut lock = read_lock(&self.config)?;
            if !lock.plugins.contains_key(id) {
                let digest = entry
                    .state
                    .lock()
                    .unwrap()
                    .package
                    .as_ref()
                    .map(|p| p.digest.clone())
                    .unwrap_or_default();
                lock.plugins.insert(
                    id.into(),
                    Locked {
                        digest,
                        enabled: false,
                    },
                );
                persist(&self.config, &lock, &guard)?;
            }
            config::ConfigStore::at(&self.config).update_locked(
                |c| {
                    c.plugins
                        .insert(id.into(), config::PluginConfig { enabled });
                    Ok(())
                },
                &guard,
            )?;
        }
        entry.state.lock().unwrap().requested_enabled = enabled;
        self.reconcile().await?;
        let state = entry.state.lock().unwrap();
        if enabled && self.optional_enabled() && !state.enabled {
            return Err(state
                .error
                .clone()
                .unwrap_or_else(|| "plugin activation failed".into()));
        }
        Ok(())
    }
    async fn change_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let entry = self.entries.get(id).ok_or("unknown external plugin")?;
        entry.reap();
        let generation = {
            let mut state = entry.state.lock().unwrap();
            if enabled && state.enabled {
                return Ok(());
            }
            state.generation += 1;
            if !enabled {
                state.enabled = false;
            }
            state.generation
        };
        let mut slot = entry.process.lock().await;
        if entry.state.lock().unwrap().generation != generation {
            return Err("plugin operation superseded".into());
        }
        let mut flight = Flight {
            process: slot.take(),
            state: &entry.state,
        };
        let mut storage_error = false;
        let result: Result<(), String> = async {
            let package =
                if enabled {
                    let package = Self::load_package(&self.config.join("plugins").join(id), id)?;
                    {
                        let mut state = entry.state.lock().unwrap();
                        state.package = Some(package.clone());
                    }
                    if package.manifest.capabilities.iter().any(|c| {
                        !matches!(c.as_str(), "model-engine" | "local-environment" | "tools" | "image-generation")
                    }) {
                        return Err("unsupported host capability grant".into());
                    }
                    if !self.local {
                        return Err(
                            "hosted plugin execution requires a sandbox execution provider".into(),
                        );
                    }
                    Some(package)
                } else {
                    None
                };
            // Only the short read/merge/write transaction holds the cross-process writer lock.
            {
                let mut storage = |error| {
                    storage_error = true;
                    error
                };
                let _guard = WriterGuard::acquire_async(self.config.clone())
                    .await
                    .map_err(&mut storage)?;
                let lock = read_lock(&self.config).map_err(&mut storage)?;
                if let Some(package) = &package {
                    if lock
                        .plugins
                        .get(id)
                        .is_some_and(|locked| locked.digest != package.digest)
                    {
                        return Err("locked package digest mismatch".into());
                    }
                }
            }
            if enabled {
                if !entry.state.lock().unwrap().enabled {
                    flight.process.take();
                }
                if flight.process.is_none() {
                    entry.state.lock().unwrap().error =
                        Some("plugin activation interrupted".into());
                    let package = package.as_ref().unwrap();
                    flight.process = Some(
                        Process::start_routed(package, Duration::from_secs(3), Some((self, id)))
                            .await?,
                    );
                }
            } else if let Some(process) = flight.process.as_mut() {
                process.shutdown().await;
                flight.process.take();
            }
            let mut state = entry.state.lock().unwrap();
            if state.generation != generation {
                flight.process.take();
                return Err("plugin operation superseded".into());
            }
            state.enabled = enabled;
            if enabled || state.package.is_some() {
                state.error = None;
            }
            state.catalog_error = None;
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            flight.process.take();
            let mut state = entry.state.lock().unwrap();
            state.enabled = false;
            state.error = Some(error.clone());
        }
        if result.is_ok() {
            *slot = flight.process.take();
        }
        if storage_error {
            result
        } else {
            Ok(())
        }
    }
    pub async fn call(&self, id: &str, method: &str, params: Value) -> Result<Value, String> {
        self.reap_dependencies();
        if method == "initialize" || method == "shutdown" {
            return Err("reserved plugin method".into());
        }
        let entry = self.entries.get(id).ok_or("unknown external plugin")?;
        let generation = {
            let state = entry.state.lock().unwrap();
            if !state.enabled {
                return Err("plugin disabled".into());
            }
            state.generation
        };
        let mut slot = entry.process.lock().await;
        {
            let state = entry.state.lock().unwrap();
            if !state.enabled || state.generation != generation {
                return Err("plugin disabled".into());
            }
        }
        let mut flight = Flight {
            process: Some(slot.take().ok_or("plugin disabled")?),
            state: &entry.state,
        };
        let process = flight.process.as_mut().unwrap();
        let result = process.call_routed(method, params, Some((self, id))).await;
        let current = {
            let state = entry.state.lock().unwrap();
            state.enabled && state.generation == generation
        };
        if matches!(process.child.try_wait(), Ok(None)) && current {
            *slot = flight.process.take();
        } else {
            flight.process.take();
            let mut state = entry.state.lock().unwrap();
            state.enabled = false;
            state.error = Some(
                result
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or_else(|| "plugin process exited".into()),
            );
        }
        drop(slot);
        self.reap_dependencies();
        if !current {
            return Err("plugin operation superseded".into());
        }
        result
    }
    pub fn ui_document(&self, id: &str) -> Result<String, String> {
        self.reap_dependencies();
        let entry = self.entries.get(id).ok_or("unknown plugin")?;
        entry.reap();
        let package = {
            let state = entry.state.lock().unwrap();
            if !self.local || !state.enabled {
                return Err("plugin disabled".into());
            }
            state.package.clone().ok_or("plugin package invalid")?
        };
        if Package::load(&package.root)?.digest != package.digest {
            return Err("package digest changed".into());
        }
        let path = package.entry(
            &package
                .manifest
                .ui
                .as_ref()
                .ok_or("plugin has no UI")?
                .entry,
        )?;
        if fs::metadata(&path).map_err(err)?.len() > 256 * 1024 {
            return Err("plugin UI too large".into());
        }
        fs::read_to_string(path).map_err(err)
    }
    pub async fn shutdown(&self) {
        let _cleanup = CleanupDisabled(self);
        for entry in self.entries.values() {
            let mut state = entry.state.lock().unwrap();
            state.generation += 1;
            state.enabled = false;
        }
        let mut order: Vec<String> = self.entries.keys().cloned().collect();
        self.dependents_first(&mut order);
        for id in order {
            let entry = &self.entries[&id];
            if let Some(process) = entry.process.lock().await.take() {
                let mut flight = Flight {
                    process: Some(process),
                    state: &entry.state,
                };
                flight.process.as_mut().unwrap().shutdown().await;
                flight.process.take();
            }
        }
    }
}

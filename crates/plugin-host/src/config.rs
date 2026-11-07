//! Validated composition settings, shared by the runtime and package host.
use super::{err, storage::*, *};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositionConfig {
    pub version: u32,
    #[serde(default = "yes")]
    pub optional_plugins_enabled: bool,
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginConfig>,
    #[serde(default)]
    pub services: ServicesConfig,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub enabled: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServicesConfig {
    pub shell: ShellConfig,
    #[serde(rename = "agent-loop")]
    pub agent_loop: AgentLoopConfig,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellConfig {
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}
impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 120_000,
            max_output_bytes: 64_000,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentLoopConfig {
    pub shutdown_timeout_ms: u64,
}
impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            shutdown_timeout_ms: 5_000,
        }
    }
}
fn yes() -> bool {
    true
}
impl Default for CompositionConfig {
    fn default() -> Self {
        Self {
            version: 1,
            optional_plugins_enabled: true,
            plugins: BTreeMap::new(),
            services: ServicesConfig::default(),
        }
    }
}
impl CompositionConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported configuration version (expected 1)".into());
        }
        for (id, plugin) in &self.plugins {
            if id.is_empty()
                || id.len() > 100
                || id.starts_with('.')
                || id.ends_with('.')
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
            {
                return Err(format!("invalid plugin id: {id}"));
            }
            if !plugin.enabled && minerva_plugin_sdk::SYSTEM_PLUGIN_IDS.contains(&id.as_str()) {
                return Err(format!("cannot disable system plugin: {id}"));
            }
        }
        if !(100..=3_600_000).contains(&self.services.shell.timeout_ms) {
            return Err("services.shell.timeoutMs must be between 100 and 3600000".into());
        }
        if !(256..=1_000_000).contains(&self.services.shell.max_output_bytes) {
            return Err("services.shell.maxOutputBytes must be between 256 and 1000000".into());
        }
        if !(100..=60_000).contains(&self.services.agent_loop.shutdown_timeout_ms) {
            return Err(
                "services.agent-loop.shutdownTimeoutMs must be between 100 and 60000".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ConfigStore {
    directory: PathBuf,
}
impl ConfigStore {
    pub fn open(config_dir: &Path) -> Result<Self, String> {
        let store = Self {
            directory: config_dir.to_owned(),
        };
        let guard = WriterGuard::acquire(config_dir)?;
        store.open_locked(&guard)?;
        Ok(store)
    }
    pub(crate) fn at(config_dir: &Path) -> Self {
        Self {
            directory: config_dir.to_owned(),
        }
    }
    fn path(&self) -> PathBuf {
        self.directory.join("minerva.conf.json")
    }
    fn error(&self, error: impl std::fmt::Display) -> String {
        format!("{}: {error}", self.path().display())
    }
    pub(crate) fn open_locked(&self, guard: &WriterGuard) -> Result<CompositionConfig, String> {
        match fs::read(self.path()) {
            Ok(bytes) => self.decode(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let legacy = read_lock(&self.directory).map_err(|e| self.error(e))?;
                let mut config = CompositionConfig {
                    optional_plugins_enabled: legacy.optional_enabled,
                    ..CompositionConfig::default()
                };
                config.plugins = legacy
                    .plugins
                    .into_iter()
                    .map(|(id, p)| (id, PluginConfig { enabled: p.enabled }))
                    .collect();
                config.plugins.insert(
                    "example".into(),
                    PluginConfig {
                        enabled: legacy.example_enabled,
                    },
                );
                let shell_path = self.directory.join("shell.json");
                match fs::read(&shell_path) {
                    Ok(bytes) => {
                        config.services.shell = serde_json::from_slice(&bytes)
                            .map_err(|e| self.error(format!("{}: {e}", shell_path.display())))?
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(self.error(format!("{}: {e}", shell_path.display()))),
                }
                self.persist_locked(&config, guard)?;
                Ok(config)
            }
            Err(e) => Err(self.error(e)),
        }
    }
    fn decode(&self, bytes: &[u8]) -> Result<CompositionConfig, String> {
        let config: CompositionConfig = serde_json::from_slice(bytes).map_err(|e| self.error(e))?;
        config.validate().map_err(|e| self.error(e))?;
        Ok(config)
    }
    pub(crate) fn read_locked(&self, _guard: &WriterGuard) -> Result<CompositionConfig, String> {
        self.decode(&fs::read(self.path()).map_err(|e| self.error(e))?)
    }
    pub fn snapshot(&self) -> Result<CompositionConfig, String> {
        let guard = WriterGuard::acquire(&self.directory)?;
        self.read_locked(&guard)
    }
    pub fn update(
        &self,
        change: impl FnOnce(&mut CompositionConfig) -> Result<(), String>,
    ) -> Result<CompositionConfig, String> {
        let guard = WriterGuard::acquire(&self.directory)?;
        self.update_locked(change, &guard)
    }
    pub(crate) fn update_locked(
        &self,
        change: impl FnOnce(&mut CompositionConfig) -> Result<(), String>,
        guard: &WriterGuard,
    ) -> Result<CompositionConfig, String> {
        let mut config = self.read_locked(guard)?;
        change(&mut config).map_err(|e| self.error(e))?;
        self.persist_locked(&config, guard)?;
        Ok(config)
    }
    fn persist_locked(
        &self,
        config: &CompositionConfig,
        _guard: &WriterGuard,
    ) -> Result<(), String> {
        config.validate().map_err(|e| self.error(e))?;
        let result = (|| {
            let mut temp = tempfile::NamedTempFile::new_in(&self.directory).map_err(err)?;
            temp.write_all(&serde_json::to_vec_pretty(config).map_err(err)?)
                .map_err(err)?;
            temp.as_file().sync_all().map_err(err)?;
            temp.persist(self.path()).map_err(err)?;
            Ok(())
        })();
        result.map_err(|e: String| self.error(e))
    }
}

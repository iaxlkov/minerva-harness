use super::*;
#[derive(Serialize, Deserialize)]
pub(crate) struct Lock {
    pub(crate) plugins: BTreeMap<String, Locked>,
    #[serde(default = "yes")]
    pub(crate) optional_enabled: bool,
    #[serde(default = "yes")]
    pub(crate) example_enabled: bool,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Locked {
    pub(crate) digest: String,
    pub(crate) enabled: bool,
}
// OS advisory lock: all package installation and state mutations are a single-writer transaction.
// The lock file remains present; the OS releases the lock if a writer crashes.
pub(crate) struct WriterGuard(fs::File);
impl WriterGuard {
    pub(crate) async fn acquire_async(config: PathBuf) -> Result<Self, String> {
        tokio::task::spawn_blocking(move || Self::acquire(&config))
            .await
            .map_err(err)?
    }
    pub(crate) fn acquire(config: &Path) -> Result<Self, String> {
        fs::create_dir_all(config).map_err(err)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.join("plugin-writer.lock"))
            .map_err(err)?;
        fs2::FileExt::lock_exclusive(&file).map_err(err)?;
        Ok(Self(file))
    }
}
impl Drop for WriterGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
pub(crate) fn read_lock(config: &Path) -> Result<Lock, String> {
    match fs::read(config.join("plugin-lock.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
        Err(e) => Err(err(e)),
    }
}

pub(crate) fn persist(config: &Path, lock: &Lock, _guard: &WriterGuard) -> Result<(), String> {
    let mut temp = tempfile::NamedTempFile::new_in(config).map_err(err)?;
    temp.write_all(&serde_json::to_vec_pretty(lock).map_err(err)?)
        .map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    temp.persist(config.join("plugin-lock.json")).map_err(err)?;
    Ok(())
}

fn yes() -> bool {
    true
}
impl Default for Lock {
    fn default() -> Self {
        Self {
            plugins: BTreeMap::new(),
            optional_enabled: true,
            example_enabled: true,
        }
    }
}

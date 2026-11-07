use super::*;
#[derive(Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub root: PathBuf,
    pub digest: String,
}
fn reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub(crate) fn files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(err)? {
        let p = entry.map_err(err)?.path();
        let meta = fs::symlink_metadata(&p).map_err(err)?;
        if reparse(&meta) {
            return Err("symlinks are forbidden in plugin packages".into());
        }
        if meta.is_dir() {
            files(root, &p, out)?
        } else if meta.is_file() {
            out.push(p.strip_prefix(root).map_err(err)?.to_path_buf())
        } else {
            return Err("unsupported package file".into());
        }
    }
    Ok(())
}
impl Package {
    pub fn load(root: &Path) -> Result<Self, String> {
        if reparse(&fs::symlink_metadata(root).map_err(err)?) {
            return Err("package root cannot be a symlink or reparse point".into());
        }
        let root = root.canonicalize().map_err(err)?;
        if fs::metadata(root.join("plugin.json")).map_err(err)?.len() > 65536 {
            return Err("manifest too large".into());
        }
        let bytes = fs::read(root.join("plugin.json")).map_err(err)?;
        if bytes.len() > 65536 {
            return Err("manifest too large".into());
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(err)?;
        manifest.validate()?;
        let mut paths = Vec::new();
        files(&root, &root, &mut paths)?;
        paths.sort();
        let mut hash = Sha256::new();
        let mut total = 0u64;
        for p in paths {
            let size = fs::metadata(root.join(&p)).map_err(err)?.len();
            if size > 256 * 1024 * 1024 - total {
                return Err("package exceeds size limit".into());
            }
            let data = fs::read(root.join(&p)).map_err(err)?;
            total += data.len() as u64;
            if total > 256 * 1024 * 1024 {
                return Err("package exceeds size limit".into());
            }
            let name = p.to_string_lossy().replace('\\', "/");
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((data.len() as u64).to_le_bytes());
            hash.update(data);
        }
        let package = Self {
            manifest,
            root,
            digest: format!("{:x}", hash.finalize()),
        };
        if let Some(b) = &package.manifest.backend {
            package.entry(&b.entry)?;
        }
        if let Some(u) = &package.manifest.ui {
            package.entry(&u.entry)?;
        }
        Ok(package)
    }
    pub fn entry(&self, entry: &str) -> Result<PathBuf, String> {
        minerva_plugin_sdk::relative_entry(entry)?;
        let p = self.root.join(entry).canonicalize().map_err(err)?;
        if !p.starts_with(&self.root) || !p.is_file() {
            return Err("entry escapes package".into());
        }
        Ok(p)
    }
}
/// Administrator-only local installation of an already built directory; executes no build scripts.
fn cleanup_stage(path: &Path, plugins: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("cleanup path has no parent")?
        .canonicalize()
        .map_err(err)?;
    let expected = plugins.canonicalize().map_err(err)?;
    if parent != expected || !path.file_name().is_some_and(|name| !name.is_empty()) {
        return Err("cleanup escaped plugin directory".into());
    }
    let resolved = path.canonicalize().map_err(err)?;
    if !resolved.starts_with(&expected) || resolved == expected {
        return Err("cleanup target escapes plugin directory".into());
    }
    fs::remove_dir_all(path).map_err(err)
}
pub fn install_local(source: &Path, config: &Path) -> Result<String, String> {
    let guard = WriterGuard::acquire(config)?;
    let package = Package::load(source)?;
    fs::create_dir_all(config.join("plugins")).map_err(err)?;
    if config.join("plugins").canonicalize().map_err(err)?.parent()
        != Some(config.canonicalize().map_err(err)?.as_path())
    {
        return Err("plugin directory escapes config directory".into());
    }
    let target = config.join("plugins").join(&package.manifest.id);
    if target.exists() {
        return Err("plugin already installed".into());
    }
    let stage = config
        .join("plugins")
        .join(format!(".stage-{}", package.manifest.id));
    fs::create_dir(&stage).map_err(err)?;
    let result = (|| {
        let mut paths = Vec::new();
        files(&package.root, &package.root, &mut paths)?;
        for p in paths {
            let dest = stage.join(&p);
            fs::create_dir_all(dest.parent().unwrap()).map_err(err)?;
            fs::copy(package.root.join(p), dest).map_err(err)?;
        }
        let copied = Package::load(&stage)?;
        if copied.digest != package.digest {
            return Err("package changed during install".into());
        }
        fs::rename(&stage, &target).map_err(err)?;
        let locked = (|| {
            let mut lock = read_lock(config)?;
            lock.plugins.insert(
                package.manifest.id.clone(),
                Locked {
                    digest: package.digest.clone(),
                    enabled: false,
                },
            );
            persist(config, &lock, &guard)
        })();
        if let Err(error) = locked {
            let _ = cleanup_stage(&target, &config.join("plugins"));
            return Err(error);
        }
        Ok(package.manifest.id.clone())
    })();
    if result.is_err() {
        let _ = cleanup_stage(&stage, &config.join("plugins"));
    }
    result
}

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
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
const LIMIT: usize = 1024 * 1024;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
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
fn files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
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
            let mut host = Host::open(config.to_path_buf(), true)?;
            host.lock.plugins.insert(
                package.manifest.id.clone(),
                Locked {
                    digest: package.digest.clone(),
                    enabled: false,
                },
            );
            host.persist(&guard)
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
#[cfg(windows)]
struct ProcessJob(isize);
#[cfg(windows)]
impl ProcessJob {
    /// std::process discards the primary-thread HANDLE. Since CREATE_SUSPENDED permits
    /// no plugin instructions before this point, the process has exactly one thread.
    /// Retrieve its documented ToolHelp thread ID, attach the Job first, and only then
    /// resume that thread. Any failure leaves it suspended and the caller kills it.
    fn resume(child: &Child) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::ToolHelp::*,
                Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
            },
        };
        let process_id = child.id().ok_or("child has no process id")?;
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(err(std::io::Error::last_os_error()));
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut exists = Thread32First(snapshot, &mut entry);
            let mut thread_id = None;
            while exists != 0 {
                if entry.th32OwnerProcessID == process_id {
                    thread_id = Some(entry.th32ThreadID);
                    break;
                }
                exists = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            let thread = OpenThread(
                THREAD_SUSPEND_RESUME,
                0,
                thread_id.ok_or("suspended primary thread not found")?,
            );
            if thread.is_null() {
                return Err(err(std::io::Error::last_os_error()));
            }
            let previous = ResumeThread(thread);
            let error = std::io::Error::last_os_error();
            CloseHandle(thread);
            if previous == u32::MAX {
                return Err(err(error));
            }
            if previous != 1 {
                return Err("primary plugin thread was not suspended exactly once".into());
            }
        }
        Ok(())
    }
    fn attach(child: &Child) -> Result<Self, String> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(
                    handle,
                    child.raw_handle().ok_or("child has no handle")? as _,
                ) == 0
            {
                let error = std::io::Error::last_os_error().to_string();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(Self(handle as isize))
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
pub struct Process {
    child: Child,
    #[cfg(unix)]
    group_id: u32,
    #[cfg(windows)]
    job: ProcessJob,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
    deadline: Duration,
}
impl Process {
    pub async fn start(package: &Package, deadline: Duration) -> Result<Self, String> {
        if Package::load(&package.root)?.digest != package.digest {
            return Err("package digest changed".into());
        }
        let backend = package.manifest.backend.as_ref().ok_or("no backend")?;
        let mut command = Command::new(package.entry(&backend.entry)?);
        command
            .current_dir(&package.root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        // CREATE_NO_WINDOW | CREATE_SUSPENDED: the backend cannot fork before Job assignment.
        command.creation_flags(0x08000000 | 0x00000004);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(err)?;
        #[cfg(windows)]
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        #[cfg(windows)]
        if let Err(error) = ProcessJob::resume(&child) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        #[cfg(unix)]
        let group_id = child.id().ok_or("child has no process id")?;
        let input = child.stdin.take().ok_or("missing stdin")?;
        let output = BufReader::new(child.stdout.take().ok_or("missing stdout")?);
        let mut process = Self {
            child,
            #[cfg(unix)]
            group_id,
            #[cfg(windows)]
            job,
            input,
            output,
            next: 0,
            deadline,
        };
        let value = process
            .call(
                "initialize",
                json!({"apiVersion":1,"pluginId":package.manifest.id}),
            )
            .await?;
        if value.get("apiVersion") != Some(&json!(1)) {
            process.stop().await;
            return Err("incompatible handshake".into());
        }
        Ok(process)
    }
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let request =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .map_err(err)?;
        if request.len() > LIMIT {
            return Err("request exceeds limit".into());
        }
        let result = timeout(self.deadline, async {
            self.input.write_all(&request).await.map_err(err)?;
            self.input.write_all(b"\n").await.map_err(err)?;
            self.input.flush().await.map_err(err)?;
            let mut line = Vec::new();
            loop {
                let buf = self.output.fill_buf().await.map_err(err)?;
                if buf.is_empty() {
                    return Err("plugin closed stdout".into());
                }
                let count = buf
                    .iter()
                    .position(|b| *b == b'\n')
                    .map(|n| n + 1)
                    .unwrap_or(buf.len());
                if line.len() + count > LIMIT {
                    return Err("response exceeds limit".into());
                }
                let done = buf[count - 1] == b'\n';
                line.extend_from_slice(&buf[..count]);
                self.output.consume(count);
                if done {
                    break;
                }
            }
            let response: Value = serde_json::from_slice(&line).map_err(err)?;
            if response.get("jsonrpc") != Some(&json!("2.0"))
                || response.get("id") != Some(&json!(id))
            {
                return Err("invalid JSON-RPC response".into());
            }
            if let Some(error) = response.get("error") {
                return Err(format!("plugin RPC error: {error}"));
            }
            response
                .get("result")
                .cloned()
                .ok_or("missing RPC result".into())
        })
        .await
        .map_err(|_| "plugin request timed out".to_string())
        .and_then(|r| r);
        if result.is_err() {
            self.stop().await;
        }
        result
    }
    pub async fn stop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job.0 as _, 1);
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.group_id as i32), libc::SIGKILL);
        }
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
    pub async fn shutdown(&mut self) {
        let _ = self.call("shutdown", Value::Null).await;
        self.stop().await;
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Lock {
    plugins: BTreeMap<String, Locked>,
}
#[derive(Serialize, Deserialize)]
struct Locked {
    digest: String,
    enabled: bool,
}
// OS advisory lock: all package installation and state mutations are a single-writer transaction.
// The lock file remains present; the OS releases the lock if a writer crashes.
struct WriterGuard(fs::File);
impl WriterGuard {
    async fn acquire_async(config: PathBuf) -> Result<Self, String> {
        tokio::task::spawn_blocking(move || Self::acquire(&config))
            .await
            .map_err(err)?
    }
    fn acquire(config: &Path) -> Result<Self, String> {
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
fn read_lock(config: &Path) -> Result<Lock, String> {
    match fs::read(config.join("plugin-lock.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
        Err(e) => Err(err(e)),
    }
}
pub struct Host {
    config: PathBuf,
    lock: Lock,
    packages: BTreeMap<String, Package>,
    processes: BTreeMap<String, Process>,
    local: bool,
}
impl Host {
    /// Hosted mode is deliberately disabled until a sandbox execution provider is integrated.
    pub fn open(config: PathBuf, local: bool) -> Result<Self, String> {
        let lock = read_lock(&config)?;
        let mut packages = BTreeMap::new();
        let dir = config.join("plugins");
        if dir.exists() {
            for entry in fs::read_dir(dir).map_err(err)? {
                let p = entry.map_err(err)?.path();
                if p.file_name().unwrap().to_string_lossy().starts_with('.') {
                    continue;
                }
                let package = Package::load(&p)?;
                if p.file_name().unwrap().to_string_lossy() != package.manifest.id {
                    return Err("package directory/id mismatch".into());
                }
                packages.insert(package.manifest.id.clone(), package);
            }
        }
        Ok(Self {
            config,
            lock,
            packages,
            processes: BTreeMap::new(),
            local,
        })
    }
    fn reap_exited(&mut self) {
        self.processes
            .retain(|_, process| matches!(process.child.try_wait(), Ok(None)));
    }
    pub fn roster(&mut self) -> Vec<(Manifest, bool)> {
        self.reap_exited();
        self.packages
            .values()
            .map(|p| {
                (
                    p.manifest.clone(),
                    self.processes.contains_key(&p.manifest.id),
                )
            })
            .collect()
    }
    pub async fn restore(&mut self) -> Result<(), String> {
        let ids: Vec<_> = self
            .lock
            .plugins
            .iter()
            .filter(|(_, v)| v.enabled)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.set_enabled(&id, true).await?;
        }
        Ok(())
    }
    fn persist(&self, _guard: &WriterGuard) -> Result<(), String> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.config).map_err(err)?;
        temp.write_all(&serde_json::to_vec_pretty(&self.lock).map_err(err)?)
            .map_err(err)?;
        temp.as_file().sync_all().map_err(err)?;
        temp.persist(self.config.join("plugin-lock.json"))
            .map_err(err)?;
        Ok(())
    }
    pub async fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.reap_exited();
        let guard = WriterGuard::acquire_async(self.config.clone()).await?;
        // Reload under the writer lock, so an installer or another host cannot be overwritten.
        self.lock = read_lock(&self.config)?;
        let package = self.packages.get(id).ok_or("unknown external plugin")?;
        if enabled {
            if !package.manifest.capabilities.is_empty() || !package.manifest.requires.is_empty() {
                return Err("capability grants and service dependencies are not supported by this local prototype".into());
            }
            if !self.local {
                return Err("hosted plugin execution requires a sandbox execution provider".into());
            }
            if let Some(locked) = self.lock.plugins.get(id) {
                if locked.digest != package.digest {
                    return Err("locked package digest mismatch".into());
                }
            }
            if !self.processes.contains_key(id) {
                let process = Process::start(package, Duration::from_secs(3)).await?;
                self.processes.insert(id.into(), process);
            }
        } else if let Some(mut process) = self.processes.remove(id) {
            process.shutdown().await;
        }
        self.lock.plugins.insert(
            id.into(),
            Locked {
                digest: package.digest.clone(),
                enabled,
            },
        );
        if let Err(e) = self.persist(&guard) {
            if let Some(mut process) = self.processes.remove(id) {
                process.stop().await;
            }
            return Err(e);
        }
        Ok(())
    }
    pub async fn call(&mut self, id: &str, method: &str, params: Value) -> Result<Value, String> {
        self.reap_exited();
        if method == "initialize" || method == "shutdown" {
            return Err("reserved plugin method".into());
        }
        let result = self
            .processes
            .get_mut(id)
            .ok_or("plugin disabled")?
            .call(method, params)
            .await;
        if result.is_err() {
            self.processes.remove(id);
        }
        result
    }
    pub async fn shutdown(&mut self) {
        for (_, mut process) in std::mem::take(&mut self.processes) {
            process.shutdown().await;
        }
    }
}

#[cfg(unix)]
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.group_id as i32), libc::SIGKILL);
        }
    }
}

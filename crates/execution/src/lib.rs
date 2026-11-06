use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub mod docker;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assignment {
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    pub assignment_id: Uuid,
    pub worker_id: Uuid,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    #[serde(rename = "memoryMiB")]
    pub memory_mib: u32,
    pub cpu_milli: u32,
    pub pids: u32,
    #[serde(rename = "diskMiB")]
    pub disk_mib: u32,
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if !(64..=16384).contains(&self.memory_mib)
            || !(10..=8000).contains(&self.cpu_milli)
            || !(8..=1024).contains(&self.pids)
            || !(96..=102400).contains(&self.disk_mib)
        {
            return Err(Error("invalid limits".into()));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Execution {
    pub argv: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
}
impl Execution {
    pub fn validate(&self) -> Result<()> {
        if self.argv.is_empty()
            || self.argv.len() > 256
            || self
                .argv
                .iter()
                .any(|v| v.len() > 65536 || v.contains('\0'))
            || self.timeout_ms == 0
            || self.timeout_ms > 300000
            || self.env.iter().any(|(k, v)| {
                !matches!(k.as_str(), "LANG" | "LC_ALL" | "TERM")
                    || v.len() > 1024
                    || v.contains('\0')
            })
        {
            return Err(Error("invalid execution request".into()));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Start {
        assignment: Assignment,
        limits: Limits,
    },
    Stop {
        assignment: Assignment,
    },
    Status {
        assignment: Assignment,
    },
    Execute {
        assignment: Assignment,
        command: Execution,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub assignment: Assignment,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
}
#[derive(Clone, Debug)]
pub struct Mounts {
    pub workspace: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
}
#[derive(Clone, Debug)]
pub struct Launch {
    pub assignment: Assignment,
    pub limits: Limits,
    pub mounts: Mounts,
}
/// A driver must stop every labelled container and confirm absence before returning.
pub trait OciDriver {
    fn set_cancellation(&mut self, _cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {}
    fn ready(&mut self) -> Result<()>;
    fn prepare(&mut self, a: &Assignment, l: &Limits) -> Result<Mounts>;
    fn stop_workspace(&mut self, tenant: Uuid, workspace: Uuid) -> Result<()>;
    fn start_host(&mut self, launch: &Launch) -> Result<()>;
    fn running(&mut self, a: &Assignment) -> Result<bool>;
    fn execute(&mut self, launch: &Launch, command: &Execution) -> Result<Output>;
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    assignment: Assignment,
    limits: Limits,
    active: bool,
}
/// Private single-writer control service. Journal is outside all container mounts.
pub struct Runner<D: OciDriver> {
    driver: D,
    journal: PathBuf,
    records: HashMap<String, Record>,
    _lock: File,
}
fn key(a: &Assignment) -> String {
    format!("{}-{}", a.tenant_id, a.workspace_id)
}
impl<D: OciDriver> Runner<D> {
    pub fn open(mut driver: D, journal: PathBuf) -> Result<Self> {
        fs::create_dir_all(&journal)?;
        checked_root(&journal)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(journal.join("runner.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error("runner already owns journal".into()))?;
        let file = journal.join("assignments.json");
        let records: HashMap<String, Record> = if file.exists() {
            serde_json::from_slice(&fs::read(&file)?)
                .map_err(|_| Error("invalid assignment journal".into()))?
        } else {
            HashMap::new()
        };
        driver.ready()?;
        // Restart recovery fences every prior assignment, including execution descendants.
        for record in records.values() {
            driver.stop_workspace(record.assignment.tenant_id, record.assignment.workspace_id)?;
        }
        let mut runner = Self {
            driver,
            journal,
            records,
            _lock: lock,
        };
        for record in runner.records.values_mut() {
            record.active = false;
        }
        runner.persist()?;
        Ok(runner)
    }
    fn persist(&self) -> Result<()> {
        let path = self.journal.join("assignments.pending");
        let mut file = File::create(&path)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&self.records).map_err(|e| Error(e.to_string()))?)?;
        file.sync_all()?;
        fs::rename(path, self.journal.join("assignments.json"))?;
        #[cfg(unix)]
        File::open(&self.journal)?.sync_all()?;
        Ok(())
    }
    fn current(&self, a: &Assignment) -> Result<Record> {
        self.records
            .get(&key(a))
            .filter(|r| r.assignment == *a && r.active)
            .cloned()
            .ok_or_else(|| Error("stale or inactive assignment".into()))
    }
    pub fn handle(&mut self, request: Request) -> Result<Response> {
        match request {
            Request::Start {
                assignment: a,
                limits,
            } => {
                limits.validate()?;
                if a.generation == 0
                    || [a.tenant_id, a.workspace_id, a.assignment_id, a.worker_id]
                        .iter()
                        .any(Uuid::is_nil)
                {
                    return Err(Error("invalid assignment".into()));
                }
                if let Some(old) = self.records.get(&key(&a)) {
                    if limits.disk_mib < old.limits.disk_mib {
                        return Err(Error("disk quota cannot shrink".into()));
                    }
                    if a.generation <= old.assignment.generation {
                        return Err(Error("generation must increase".into()));
                    }
                }
                self.driver.stop_workspace(a.tenant_id, a.workspace_id)?;
                // Persist the fence BEFORE any mount or creation. A failed start burns its generation.
                self.records.insert(
                    key(&a),
                    Record {
                        assignment: a.clone(),
                        limits: limits.clone(),
                        active: false,
                    },
                );
                self.persist()?;
                let mounts = self.driver.prepare(&a, &limits)?;
                let launch = Launch {
                    assignment: a.clone(),
                    limits: limits.clone(),
                    mounts,
                };
                if let Err(error) = self.driver.start_host(&launch) {
                    self.driver.stop_workspace(a.tenant_id, a.workspace_id)?;
                    return Err(error);
                }
                self.records.get_mut(&key(&a)).unwrap().active = true;
                self.persist()?;
                Ok(Response {
                    assignment: a,
                    running: true,
                    output: None,
                })
            }
            Request::Stop { assignment: a } => {
                self.current(&a)?;
                self.driver.stop_workspace(a.tenant_id, a.workspace_id)?;
                self.records.get_mut(&key(&a)).unwrap().active = false;
                self.persist()?;
                Ok(Response {
                    assignment: a,
                    running: false,
                    output: None,
                })
            }
            Request::Status { assignment: a } => {
                self.current(&a)?;
                let running = self.driver.running(&a)?;
                Ok(Response {
                    assignment: a,
                    running,
                    output: None,
                })
            }
            Request::Execute {
                assignment: a,
                command,
            } => {
                command.validate()?;
                let record = self.current(&a)?;
                if !self.driver.running(&a)? {
                    return Err(Error("host not running".into()));
                }
                let mounts = self.driver.prepare(&a, &record.limits)?;
                let output = self.driver.execute(
                    &Launch {
                        assignment: a.clone(),
                        limits: record.limits,
                        mounts,
                    },
                    &command,
                );
                let output = match output {
                    Ok(output) => output,
                    Err(error) => {
                        // A failed cleanup may leave a user container alive. Fence the
                        // assignment before any further work, then stop the whole workspace.
                        self.records.get_mut(&key(&a)).unwrap().active = false;
                        self.persist()?;
                        self.driver.stop_workspace(a.tenant_id, a.workspace_id)?;
                        return Err(error);
                    }
                };
                Ok(Response {
                    assignment: a,
                    running: true,
                    output: Some(output),
                })
            }
        }
    }
    pub fn set_cancellation(&mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.driver.set_cancellation(cancel);
    }
    pub fn into_driver(self) -> D {
        self.driver
    }
}
/// Hosted providers never spawn commands in the worker process.
pub trait ExecutionProvider {
    fn execute(&mut self, assignment: Assignment, command: Execution) -> Result<Output>;
}
impl<D: OciDriver> ExecutionProvider for Runner<D> {
    fn execute(&mut self, assignment: Assignment, command: Execution) -> Result<Output> {
        self.handle(Request::Execute {
            assignment,
            command,
        })?
        .output
        .ok_or_else(|| Error("missing execution output".into()))
    }
}
pub fn checked_root(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() {
        return Err(Error("absolute root required".into()));
    }
    for part in root.ancestors() {
        if fs::symlink_metadata(part)?.file_type().is_symlink() {
            return Err(Error("symlink in storage root".into()));
        }
    }
    Ok(fs::canonicalize(root)?)
}

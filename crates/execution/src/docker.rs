//! Linux Docker adapter. Every container explicitly selects runsc; no alternate runtime.
use crate::*;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DockerConfig {
    pub root: PathBuf,
    pub host_image: String,
    pub execution_image: String,
    pub control_network: String,
}
pub trait ControlCommand {
    fn output(&mut self, program: &str, args: &[String]) -> Result<(String, String)> {
        Ok((self.run(program, args)?, String::new()))
    }

    fn run(&mut self, program: &str, args: &[String]) -> Result<String>;
}
pub struct SystemCommand;
impl ControlCommand for SystemCommand {
    fn output(&mut self, program: &str, args: &[String]) -> Result<(String, String)> {
        capture(program, args)
    }

    fn run(&mut self, program: &str, args: &[String]) -> Result<String> {
        run(program, args)
    }
}
pub struct DockerDriver {
    config: DockerConfig,
    control: std::cell::RefCell<Box<dyn ControlCommand>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl DockerDriver {
    pub fn new(config: DockerConfig) -> Result<Self> {
        for image in [&config.host_image, &config.execution_image] {
            let (_, digest) = image
                .rsplit_once("@sha256:")
                .ok_or_else(|| Error("operator-pinned image digest required".into()))?;
            if digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                || image.starts_with('-')
                || image.contains(char::is_whitespace)
            {
                return Err(Error("invalid pinned image".into()));
            }
        }
        if !config.control_network.starts_with("minerva-control-")
            || !config
                .control_network
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(Error("private control network required".into()));
        }
        checked_root(&config.root)?;
        Ok(Self {
            config,
            control: std::cell::RefCell::new(Box::new(SystemCommand)),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    pub fn with_control(config: DockerConfig, control: Box<dyn ControlCommand>) -> Result<Self> {
        let mut result = Self::new(config)?;
        result.control = std::cell::RefCell::new(control);
        Ok(result)
    }
    fn control(&self, program: &str, args: &[String]) -> Result<String> {
        self.control.borrow_mut().run(program, args)
    }
    fn docker(&self, args: &[String]) -> Result<String> {
        let mut base = vec!["--host=unix:///var/run/docker.sock".into()];
        base.extend_from_slice(args);
        self.control("docker", &base)
    }
    fn names(&self, t: Uuid, w: Uuid) -> Result<Vec<String>> {
        let output = self.docker(&[
            "ps".into(),
            "-aq".into(),
            "--no-trunc".into(),
            "--filter".into(),
            format!("label=minerva.tenant={t}"),
            "--filter".into(),
            format!("label=minerva.workspace={w}"),
        ])?;
        let ids: Vec<String> = output.lines().map(str::to_owned).collect();
        if ids
            .iter()
            .any(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(Error("unexpected container ID".into()));
        }
        Ok(ids)
    }
    fn name(a: &Assignment) -> String {
        format!(
            "minerva-host-{}-{}-{}",
            a.tenant_id, a.workspace_id, a.generation
        )
    }
    fn create(
        &self,
        l: &Launch,
        name: &str,
        host: bool,
        command: Option<&Execution>,
    ) -> Result<()> {
        l.limits.validate()?;
        let mut args = vec![
            "create".into(),
            "--runtime=runsc".into(),
            "--pull=never".into(),
            format!("--name={name}"),
            "--restart=no".into(),
            "--init".into(),
            "--read-only".into(),
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges:true".into(),
            "--user=65532:65532".into(),
            format!("--memory={}m", l.limits.memory_mib),
            format!("--memory-swap={}m", l.limits.memory_mib),
            format!("--cpus={:.3}", l.limits.cpu_milli as f64 / 1000.0),
            format!("--pids-limit={}", l.limits.pids),
            "--ulimit=nofile=1024:1024".into(),
            "--log-driver=local".into(),
            "--log-opt=max-size=1m".into(),
            "--log-opt=max-file=1".into(),
            format!(
                "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size={}m,uid=65532,gid=65532",
                l.limits.disk_mib / 6
            ),
            "--env=HOME=/tmp/home".into(),
            "--env=PATH=/usr/local/bin:/usr/bin:/bin".into(),
            "--workdir=/workspace".into(),
            format!("--label=minerva.tenant={}", l.assignment.tenant_id),
            format!("--label=minerva.workspace={}", l.assignment.workspace_id),
            format!("--label=minerva.assignment={}", l.assignment.assignment_id),
            format!("--label=minerva.generation={}", l.assignment.generation),
            format!(
                "--network={}",
                if host {
                    &self.config.control_network
                } else {
                    "none"
                }
            ),
        ];
        args.extend(["--mount".into(), mount(&l.mounts.workspace, "/workspace")?]);
        if host {
            args.extend([
                "--mount".into(),
                mount(&l.mounts.data, "/data")?,
                "--mount".into(),
                mount(&l.mounts.config, "/config")?,
                format!("--env=MINERVA_TENANT_ID={}", l.assignment.tenant_id),
                format!("--env=MINERVA_WORKSPACE_ID={}", l.assignment.workspace_id),
                format!("--env=MINERVA_ASSIGNMENT_ID={}", l.assignment.assignment_id),
                format!("--env=MINERVA_WORKER_ID={}", l.assignment.worker_id),
                format!("--env=MINERVA_GENERATION={}", l.assignment.generation),
            ]);
        }
        if let Some(c) = command {
            for (k, v) in &c.env {
                args.push(format!("--env={k}={v}"));
            }
        }
        args.push(if host {
            self.config.host_image.clone()
        } else {
            self.config.execution_image.clone()
        });
        if let Some(c) = command {
            args.extend(c.argv.clone());
        }
        self.docker(&args)?;
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<()> {
        self.docker(&["rm".into(), "--force".into(), name.into()])?;
        Ok(())
    }
    fn state(&self, name: &str) -> Result<serde_json::Value> {
        serde_json::from_str(&self.docker(&[
            "inspect".into(),
            "--format={{json .State}}".into(),
            name.into(),
        ])?)
        .map_err(|_| Error("invalid container state".into()))
    }
}
fn mount(source: &Path, target: &str) -> Result<String> {
    let source = checked_root(source)?;
    let source = source
        .to_str()
        .ok_or_else(|| Error("non UTF-8 mount".into()))?;
    if source.contains([',', '\n', '\r']) {
        return Err(Error("unsafe mount path".into()));
    }
    Ok(format!(
        "type=bind,src={source},dst={target},bind-propagation=rprivate"
    ))
}
impl OciDriver for DockerDriver {
    fn set_cancellation(&mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.cancel = cancel;
    }
    fn ready(&mut self) -> Result<()> {
        if !cfg!(target_os = "linux") {
            return Err(Error("hosted runner requires Linux/runsc".into()));
        }
        let info: serde_json::Value =
            serde_json::from_str(&self.docker(&["info".into(), "--format={{json .}}".into()])?)
                .map_err(|_| Error("invalid Docker info".into()))?;
        if info["OSType"] != "linux" || info["Runtimes"].get("runsc").is_none() {
            return Err(Error("runsc unavailable".into()));
        }
        for image in [&self.config.host_image, &self.config.execution_image] {
            self.docker(&["image".into(), "inspect".into(), image.clone()])?;
        }
        let net: serde_json::Value = serde_json::from_str(&self.docker(&[
            "network".into(),
            "inspect".into(),
            self.config.control_network.clone(),
        ])?)
        .map_err(|_| Error("invalid network".into()))?;
        if net[0]["Internal"] != true || net[0]["Driver"] != "bridge" {
            return Err(Error("control network must be internal bridge".into()));
        }
        self.control(
            "btrfs",
            &[
                "filesystem".into(),
                "show".into(),
                self.config.root.to_string_lossy().into_owned(),
            ],
        )?;
        Ok(())
    }
    fn prepare(&mut self, a: &Assignment, l: &Limits) -> Result<Mounts> {
        let root = checked_root(&self.config.root)?;
        let tenant = root.join(a.tenant_id.to_string());
        fs::create_dir_all(&tenant)?;
        checked_root(&tenant)?;
        self.control(
            "chmod",
            &["0700".into(), tenant.to_string_lossy().into_owned()],
        )?;
        let base = tenant.join(a.workspace_id.to_string());
        if !base.exists() {
            self.control(
                "btrfs",
                &[
                    "subvolume".into(),
                    "create".into(),
                    base.to_string_lossy().into_owned(),
                ],
            )?;
        }
        checked_root(&base)?;
        self.control(
            "chmod",
            &["0700".into(), base.to_string_lossy().into_owned()],
        )?;
        self.control(
            "btrfs",
            &[
                "subvolume".into(),
                "show".into(),
                base.to_string_lossy().into_owned(),
            ],
        )?;
        // One qgroup covers all three persistent directories and ALL snapshots/references.
        self.control(
            "btrfs",
            &[
                "qgroup".into(),
                "limit".into(),
                format!("{}M", l.disk_mib * 2 / 3),
                base.to_string_lossy().into_owned(),
            ],
        )?;
        let mounts = Mounts {
            workspace: base.join("workspace"),
            data: base.join("data"),
            config: base.join("config"),
        };
        for path in [&mounts.workspace, &mounts.data, &mounts.config] {
            fs::create_dir_all(path)?;
            checked_root(path)?;
            self.control(
                "chmod",
                &["0700".into(), path.to_string_lossy().into_owned()],
            )?;
            self.control(
                "chown",
                &["65532:65532".into(), path.to_string_lossy().into_owned()],
            )?;
        }
        Ok(mounts)
    }
    fn stop_workspace(&mut self, t: Uuid, w: Uuid) -> Result<()> {
        for name in self.names(t, w)? {
            self.remove(&name)?;
        }
        if !self.names(t, w)?.is_empty() {
            return Err(Error("workspace stop not confirmed".into()));
        }
        Ok(())
    }
    fn start_host(&mut self, l: &Launch) -> Result<()> {
        let name = Self::name(&l.assignment);
        self.create(l, &name, true, None)?;
        self.docker(&["start".into(), name.clone()])?;
        if self.state(&name)?["Running"] != true {
            return Err(Error("host failed to start".into()));
        }
        Ok(())
    }
    fn running(&mut self, a: &Assignment) -> Result<bool> {
        Ok(self.state(&Self::name(a))?["Running"] == true)
    }
    fn execute(&mut self, l: &Launch, c: &Execution) -> Result<Output> {
        c.validate()?;
        let cancelled = || Output {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
        };
        if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(cancelled());
        }
        let name = format!("minerva-exec-{}", Uuid::new_v4());
        let result = (|| {
            self.create(l, &name, false, Some(c))?;
            if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(cancelled());
            }
            self.docker(&["start".into(), name.clone()])?;
            let start = Instant::now();
            let (code, timed_out) = loop {
                if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(cancelled());
                }
                let state = self.state(&name)?;
                if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(cancelled());
                }
                if state["Running"] == false {
                    break (state["ExitCode"].as_i64().unwrap_or(-1) as i32, false);
                }
                if start.elapsed() >= Duration::from_millis(c.timeout_ms) {
                    self.docker(&["kill".into(), name.clone()])?;
                    break (-1, true);
                }
                thread::sleep(Duration::from_millis(25));
            };
            let (stdout, stderr) = self.control.borrow_mut().output(
                "docker",
                &[
                    "--host=unix:///var/run/docker.sock".into(),
                    "logs".into(),
                    name.clone(),
                ],
            )?;
            if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(cancelled());
            }
            Ok(Output {
                exit_code: code,
                stdout,
                stderr,
                timed_out,
            })
        })();
        // docker rm -f kills the container cgroup, including detached/orphan descendants.
        let cleanup = self.remove(&name);
        match (result, cleanup) {
            (_, Err(e)) => Err(e),
            (Ok(_), Ok(())) if self.cancel.load(std::sync::atomic::Ordering::Acquire) => {
                Ok(cancelled())
            }
            (r, Ok(())) => r,
        }
    }
}
fn run(program: &str, args: &[String]) -> Result<String> {
    Ok(capture(program, args)?.0)
}
fn capture(program: &str, args: &[String]) -> Result<(String, String)> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    fn drain(mut stream: impl Read) -> Vec<u8> {
        let mut result = Vec::new();
        let mut buf = [0; 8192];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            let take = n.min(1024 * 1024 - result.len());
            result.extend_from_slice(&buf[..take]);
        }
        result
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || drain(stdout));
    let err = thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error("container control command timed out".into()));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = out
        .join()
        .map_err(|_| Error("output reader failed".into()))?;
    let stderr = err
        .join()
        .map_err(|_| Error("output reader failed".into()))?;
    if !status.success() {
        return Err(Error(format!(
            "{program} control operation failed ({status})"
        )));
    }
    Ok((
        String::from_utf8_lossy(&output).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}

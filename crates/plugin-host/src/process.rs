use super::*;
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
    pub(crate) child: Child,
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
        Self::start_routed(package, deadline, None).await
    }
    pub(crate) async fn start_routed(
        package: &Package,
        deadline: Duration,
        router: Option<(&Host, &str)>,
    ) -> Result<Self, String> {
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
        if package
            .manifest
            .capabilities
            .iter()
            .any(|c| c == "local-environment")
        {
            for name in [
                "PATH",
                "HOME",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "SYSTEMROOT",
                "TEMP",
                "TMP",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
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
        let data_dir = package
            .root
            .parent()
            .ok_or("package has no parent")?
            .join(".data")
            .join(&package.manifest.id);
        fs::create_dir_all(&data_dir).map_err(err)?;
        let value = process
            .call_routed(
                "initialize",
                json!({"apiVersion":1,"pluginId":package.manifest.id,"dataDir":data_dir}),
                router,
            )
            .await?;
        if value.get("apiVersion") != Some(&json!(1)) {
            process.stop().await;
            return Err("incompatible handshake".into());
        }
        Ok(process)
    }
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call_routed(method, params, None).await
    }
    pub(crate) async fn call_routed(
        &mut self,
        method: &str,
        params: Value,
        router: Option<(&Host, &str)>,
    ) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let request =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .map_err(err)?;
        let request_limit = if method == "engine.start" {
            32 * 1024 * 1024
        } else {
            LIMIT
        };
        if request.len() > request_limit {
            return Err("request exceeds limit".into());
        }
        let mut remote_error = false;
        let deadline = if method == "images.available" { Duration::from_secs(12) } else { self.deadline };
        let result = timeout(deadline, async {
            self.input.write_all(&request).await.map_err(err)?;
            self.input.write_all(b"\n").await.map_err(err)?;
            self.input.flush().await.map_err(err)?;
            for _ in 0..64 {
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
            if response.get("method").is_some() {
                if response.get("jsonrpc") != Some(&json!("2.0")) || !response.get("id").is_some_and(|id| id.is_u64() || id.is_string()) { return Err("invalid service request".into()); }
                let result = if response["method"] != "services.call" { Err("unsupported host method".into()) }
                    else if let Some((host, consumer)) = router { host.route_service(consumer, response["params"].clone()).await }
                    else { Err("host services unavailable".into()) };
                let reply = match result {
                    Ok(value) => json!({"jsonrpc":"2.0","id":response["id"],"result":value}),
                    Err(error) => json!({"jsonrpc":"2.0","id":response["id"],"error":{"code":-32602,"message":error}}),
                };
                let bytes = serde_json::to_vec(&reply).map_err(err)?;
                if bytes.len() > LIMIT { return Err("service response exceeds limit".into()); }
                self.input.write_all(&bytes).await.map_err(err)?;
                self.input.write_all(b"\n").await.map_err(err)?;
                self.input.flush().await.map_err(err)?;
                continue;
            }
            if response.get("jsonrpc") != Some(&json!("2.0"))
                || response.get("id") != Some(&json!(id))
            {
                return Err("invalid JSON-RPC response".into());
            }
            if let Some(error) = response.get("error") {
                remote_error = true;
                return Err(format!("plugin RPC error: {error}"));
            }
            return response
                .get("result")
                .cloned()
                .ok_or("missing RPC result".into());
            }
            Err("too many service requests".into())
        })
        .await
        .map_err(|_| "plugin request timed out".to_string())
        .and_then(|r| r);
        if result.is_err() && !remote_error {
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

#[cfg(unix)]
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.group_id as i32), libc::SIGKILL);
        }
    }
}

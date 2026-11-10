//! Process ownership outlives the shell leader and kills descendants on drop.
#[cfg(windows)]
pub struct ProcessTree(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl ProcessTree {
    pub fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows_sys::Win32::System::JobObjects::*;
        // The child is waiting on piped stdin: no user command runs until the
        // process is in this job, including very short commands spawning children.
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let owned = std::os::windows::io::OwnedHandle::from_raw_handle(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                owned.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let process = child.raw_handle().ok_or("missing shell process handle")?;
            if AssignProcessToJobObject(owned.as_raw_handle(), process) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(Self(owned))
        }
    }
    pub fn kill(&self) {
        use std::os::windows::io::AsRawHandle;
        // The owned job handle is valid for the whole call.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}
#[cfg(not(windows))]
pub struct ProcessTree(u32);
#[cfg(not(windows))]
impl ProcessTree {
    pub fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        Ok(Self(child.id().ok_or("missing shell process id")?))
    }
    pub fn kill(&self) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}
#[cfg(not(windows))]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}

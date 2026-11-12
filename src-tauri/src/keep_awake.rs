//! Holds an OS sleep inhibitor while the frontend reports agent work.
//! Each platform's own tool keeps the lock and exits with Minerva, so a crash never leaves it behind.
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[derive(Default)]
pub struct KeepAwake(Mutex<Option<Child>>);

fn inhibitor() -> Command {
    let pid = std::process::id().to_string();
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("caffeinate");
        command.args(["-i", "-w", &pid]);
        command
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let script = format!(
            "$p=Add-Type -Name Power -Namespace Minerva -PassThru -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint flags);'; \
             [void]$p::SetThreadExecutionState(0x80000001); Wait-Process -Id {pid}"
        );
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut command = Command::new("systemd-inhibit");
        command.args([
            "--what=sleep:idle",
            "--who=Minerva",
            "--why=Агент выполняет задачу",
            "--mode=block",
            "tail",
            &format!("--pid={pid}"),
            "-f",
            "/dev/null",
        ]);
        command
    }
}

impl KeepAwake {
    pub fn set(&self, enabled: bool) -> Result<(), String> {
        let mut held = self.0.lock().map_err(|e| e.to_string())?;
        // A helper that already exited (e.g. inhibition refused) no longer holds the lock.
        if let Some(child) = held.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                *held = None;
            }
        }
        match (enabled, held.is_some()) {
            (true, false) => {
                let child = inhibitor()
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("Не удалось запретить сон: {e}"))?;
                *held = Some(child);
            }
            (false, true) => {
                if let Some(mut child) = held.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[tauri::command]
pub fn set_keep_awake(state: tauri::State<'_, KeepAwake>, enabled: bool) -> Result<(), String> {
    state.set(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggling_twice_is_idempotent_and_releases() {
        let keep = KeepAwake::default();
        // The inhibitor may be unavailable in CI; either way release must leave nothing held.
        let _ = keep.set(true);
        let _ = keep.set(true);
        keep.set(false).unwrap();
        assert!(keep.0.lock().unwrap().is_none());
    }
}

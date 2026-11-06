//! Built-in shell service: persistent limits and explicit one-shot commands.
use cordis::{Context, Plugin, Service};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    fs::{self, File},
    io::Read,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ShellSettings {
    #[ts(type = "number")]
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}
impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 120_000,
            max_output_bytes: 64_000,
        }
    }
}
impl ShellSettings {
    fn validate(&self) -> Result<(), String> {
        if !(100..=3_600_000).contains(&self.timeout_ms) {
            return Err("Тайм-аут должен быть от 100 до 3600000 мс.".into());
        }
        if !(256..=1_000_000).contains(&self.max_output_bytes) {
            return Err("Лимит вывода должен быть от 256 до 1000000 байт.".into());
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ShellResult {
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
    stdout_file: Option<String>,
    stderr_file: Option<String>,
}

pub struct ShellService {
    path: PathBuf,
    settings: Mutex<ShellSettings>,
}
impl Service for ShellService {
    const NAME: &'static str = "studio.shell";
}
impl ShellService {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let settings = match fs::read(&path) {
            Ok(data) => {
                let value: ShellSettings = serde_json::from_slice(&data)
                    .map_err(|e| format!("Ошибка чтения настроек оболочки: {e}"))?;
                value.validate()?;
                value
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ShellSettings::default(),
            Err(e) => return Err(format!("Ошибка чтения настроек оболочки: {e}")),
        };
        Ok(Self {
            path,
            settings: Mutex::new(settings),
        })
    }
    pub async fn settings(&self) -> ShellSettings {
        self.settings.lock().await.clone()
    }
    pub async fn save(&self, value: ShellSettings) -> Result<ShellSettings, String> {
        value.validate()?;
        let mut current = self.settings.lock().await;
        let dir = self.path.parent().ok_or("Не задан каталог настроек.")?;
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut temporary = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut temporary, &value).map_err(|e| e.to_string())?;
        temporary
            .persist(&self.path)
            .map_err(|e| format!("Не удалось сохранить настройки: {e}"))?;
        *current = value.clone();
        Ok(value)
    }
    pub async fn run(&self, command: String) -> Result<ShellResult, String> {
        if command.trim().is_empty() {
            return Err("Введите команду.".into());
        }
        let settings = self.settings().await;
        let out = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let err = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let stdout = out.reopen().map_err(|e| e.to_string())?;
        let stderr = err.reopen().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("powershell.exe");
            let script = format!("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); $OutputEncoding = [Console]::OutputEncoding; {command}");
            c.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ]);
            c.creation_flags(0x08000000);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &command]);
            c
        };
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Не удалось запустить оболочку: {e}"))?;
        let (status, timed_out) =
            match tokio::time::timeout(Duration::from_millis(settings.timeout_ms), child.wait())
                .await
            {
                Ok(result) => (Some(result.map_err(|e| e.to_string())?), false),
                Err(_) => {
                    child
                        .kill()
                        .await
                        .map_err(|e| format!("Не удалось завершить оболочку: {e}"))?;
                    (None, true)
                }
            };
        fn capture(
            temp: tempfile::NamedTempFile,
            limit: usize,
        ) -> Result<(String, Option<String>), String> {
            let size = temp.as_file().metadata().map_err(|e| e.to_string())?.len();
            let mut data = Vec::new();
            File::open(temp.path())
                .map_err(|e| e.to_string())?
                .take(limit as u64)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            let path = if size > limit as u64 {
                let (_, path) = temp.keep().map_err(|e| e.to_string())?;
                Some(path.to_string_lossy().into_owned())
            } else {
                None
            };
            Ok((String::from_utf8_lossy(&data).into_owned(), path))
        }
        let (stdout, stdout_file) = capture(out, settings.max_output_bytes)?;
        let (stderr, stderr_file) = capture(err, settings.max_output_bytes)?;
        Ok(ShellResult {
            exit_code: status.and_then(|s| s.code()),
            timed_out,
            stdout,
            stderr,
            stdout_file,
            stderr_file,
        })
    }
}

pub struct ShellPlugin(pub Arc<ShellService>);
impl Plugin for ShellPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _publication = ctx.provide(self.0.clone())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn saves_settings_and_runs_with_bounded_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell.json");
        let shell = ShellService::new(path.clone()).unwrap();
        let settings = ShellSettings {
            timeout_ms: 5000,
            max_output_bytes: 256,
        };
        shell.save(settings).await.unwrap();
        assert_eq!(
            ShellService::new(path)
                .unwrap()
                .settings()
                .await
                .max_output_bytes,
            256
        );
        #[cfg(windows)]
        let command = "[Console]::Out.Write('x' * 400)";
        #[cfg(not(windows))]
        let command = "printf '%0400d' 0";
        let result = shell.run(command.into()).await.unwrap();
        assert_eq!(result.stdout.len(), 256);
        let spill = result.stdout_file.unwrap();
        assert_eq!(fs::metadata(&spill).unwrap().len(), 400);
        fs::remove_file(spill).unwrap();
        assert!(shell
            .save(ShellSettings {
                timeout_ms: 0,
                max_output_bytes: 256
            })
            .await
            .is_err());
        #[cfg(windows)]
        {
            let unicode = shell
                .run("[Console]::Out.Write([char]0x041F)".into())
                .await
                .unwrap();
            assert_eq!(unicode.stdout, "П");
            shell
                .save(ShellSettings {
                    timeout_ms: 100,
                    max_output_bytes: 256,
                })
                .await
                .unwrap();
            assert!(
                shell
                    .run("Start-Sleep -Seconds 2".into())
                    .await
                    .unwrap()
                    .timed_out
            );
        }
    }
}

//! Cordis owns plugin lifetimes; the built-in manager publishes the roster service.
use crate::codex::{CodexPlugin, CodexService};
use crate::providers::{ProviderPlugin, ProviderService};
use crate::shell::{ShellPlugin, ShellService};
use cordis::event::{observer_sync, ListenerRegistrationError};
use cordis::{Context, Event, FiberHandle, FiberState, Plugin, PreparedPlugin, Routing, Service};
use serde::Serialize;
use std::path::PathBuf;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

struct HealthCheck;
impl Event for HealthCheck {
    const NAME: &'static str = "studio/plugin-health-check";
    type Args = ();
    type Output = ();
}

struct ExamplePlugin {
    counter: Arc<AtomicUsize>,
}
impl Plugin for ExamplePlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = ListenerRegistrationError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let counter = self.counter.clone();
        ctx.on::<HealthCheck, _>(observer_sync(move |_, ()| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok::<_, Infallible>(())
        }))?;
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    enabled: bool,
    can_toggle: bool,
    origin: &'static str,
}

struct ManagerService {
    context: Context,
    example: Mutex<Option<FiberHandle>>,
    example_events: Arc<AtomicUsize>,
}
impl Service for ManagerService {
    const NAME: &'static str = "studio.plugin-manager";
}

impl ManagerService {
    async fn start_example(&self) -> Result<FiberHandle, String> {
        self.context
            .spawn(PreparedPlugin::from_input(
                ExamplePlugin {
                    counter: self.example_events.clone(),
                },
                (),
            ))
            .await
            .map_err(|error| format!("Не удалось запустить плагин: {error}"))
    }
    async fn list(&self) -> Vec<PluginInfo> {
        let example = self.example.lock().await;
        vec![
            PluginInfo {
                id: "plugin-manager",
                name: "Менеджер плагинов",
                description: "Предоставляет сервис управления встроенными плагинами.",
                enabled: true,
                can_toggle: false,
                origin: "builtin",
            },
            PluginInfo {
                id: "providers",
                name: "Провайдеры моделей",
                description: "Конфигурация и подключение моделей.",
                enabled: self.context.try_service::<ProviderService>().is_ok(),
                can_toggle: false,
                origin: "builtin",
            },
            PluginInfo {
                id: "codex",
                name: "Codex",
                description: "Подключение Codex App Server.",
                enabled: self.context.try_service::<CodexService>().is_ok(),
                can_toggle: false,
                origin: "builtin",
            },
            PluginInfo {
                id: "shell",
                name: "Командная оболочка",
                description: "Запуск команд и ограничения вывода.",
                enabled: self.context.try_service::<ShellService>().is_ok(),
                can_toggle: false,
                origin: "builtin",
            },
            PluginInfo {
                id: "example",
                name: "Тестовый плагин",
                description: "Проверяет запуск, события и отключение в Cordis.",
                enabled: example
                    .as_ref()
                    .is_some_and(|handle| handle.state() == FiberState::Active),
                can_toggle: true,
                origin: "builtin",
            },
        ]
    }
    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        if id != "example" {
            return Err("Управление этим плагином недоступно.".into());
        }
        let mut example = self.example.lock().await;
        if enabled && example.is_none() {
            *example = Some(self.start_example().await?);
        } else if !enabled {
            if let Some(handle) = example.as_ref() {
                handle
                    .dispose()
                    .await
                    .map_err(|error| format!("Не удалось отключить плагин: {error}"))?;
                *example = None;
            }
        }
        drop(example);
        Ok(self.list().await)
    }
    async fn shutdown(&self) {
        if let Some(handle) = self.example.lock().await.take() {
            let _ = handle.dispose().await;
        }
    }
}

struct ManagerPlugin {
    service: Arc<ManagerService>,
}
impl Plugin for ManagerPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _publication = ctx.provide(self.service.clone())?;
        Ok(())
    }
}

pub struct PluginManager {
    context: Context,
    manager: FiberHandle,
    shell: FiberHandle,
    providers: FiberHandle,
    codex: FiberHandle,
    #[cfg(test)]
    example_events: Arc<AtomicUsize>,
}
impl PluginManager {
    pub async fn start(settings_path: PathBuf) -> Result<Self, String> {
        // Read fallible configuration before publishing any lifecycle effects.
        let shell_service = Arc::new(ShellService::new(settings_path.clone())?);
        let provider_service = Arc::new(ProviderService::new(
            settings_path.with_file_name("providers.json"),
        )?);
        let context = Context::new();
        let example_events = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(ManagerService {
            context: context.clone(),
            example: Mutex::new(None),
            example_events: example_events.clone(),
        });
        let mut started = Vec::new();
        let result = async {
            let manager = context
                .spawn(PreparedPlugin::from_input(
                    ManagerPlugin {
                        service: service.clone(),
                    },
                    (),
                ))
                .await
                .map_err(|e| e.to_string())?;
            started.push(manager.clone());
            *service.example.lock().await = Some(service.start_example().await?);
            let shell = context
                .spawn(PreparedPlugin::from_input(ShellPlugin(shell_service), ()))
                .await
                .map_err(|e| e.to_string())?;
            started.push(shell.clone());
            let providers = context
                .spawn(PreparedPlugin::from_input(
                    ProviderPlugin(provider_service),
                    (),
                ))
                .await
                .map_err(|e| e.to_string())?;
            started.push(providers.clone());
            let codex = context
                .spawn(PreparedPlugin::from_input(
                    CodexPlugin(Arc::new(CodexService::default())),
                    (),
                ))
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((manager, shell, providers, codex))
        }
        .await;
        let (manager, shell, providers, codex) = match result {
            Ok(value) => value,
            Err(error) => {
                service.shutdown().await;
                for handle in started.iter().rev() {
                    let _ = handle.dispose().await;
                }
                return Err(error);
            }
        };
        Ok(Self {
            context,
            manager,
            shell,
            providers,
            codex,
            #[cfg(test)]
            example_events,
        })
    }
    fn service(&self) -> Result<Arc<ManagerService>, String> {
        self.context
            .try_service::<ManagerService>()
            .map_err(|error| format!("Менеджер плагинов недоступен: {error}"))
    }
    pub async fn list(&self) -> Result<Vec<PluginInfo>, String> {
        Ok(self.service()?.list().await)
    }
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        self.service()?.set_enabled(id, enabled).await
    }
    pub fn shell(&self) -> Result<Arc<ShellService>, String> {
        self.context
            .try_service::<ShellService>()
            .map_err(|error| format!("Оболочка недоступна: {error}"))
    }
    pub fn providers(&self) -> Result<Arc<ProviderService>, String> {
        self.context
            .try_service::<ProviderService>()
            .map_err(|e| e.to_string())
    }
    pub fn codex(&self) -> Result<Arc<CodexService>, String> {
        self.context
            .try_service::<CodexService>()
            .map_err(|e| e.to_string())
    }
    pub async fn shutdown(&self) {
        let _ = self.codex.dispose().await;
        let _ = self.providers.dispose().await;
        let _ = self.shell.dispose().await;
        if let Ok(service) = self.service() {
            service.shutdown().await;
        }
        let _ = self.manager.dispose().await;
    }
    #[cfg(test)]
    async fn probe(&self) -> usize {
        self.context
            .emit::<HealthCheck>(Routing::Unscoped, ())
            .await
            .unwrap();
        self.example_events.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn plugin_manager_publishes_service_and_cleans_up_lifetimes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = PluginManager::start(temp.path().join("shell.json"))
            .await
            .unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.set_enabled("example", false).await.unwrap();
        assert_eq!(manager.probe().await, 1);
        manager.set_enabled("example", true).await.unwrap();
        assert_eq!(manager.probe().await, 2);
        assert!(manager.set_enabled("plugin-manager", false).await.is_err());
        manager.shutdown().await;
        assert_eq!(manager.probe().await, 2);
        assert!(manager.service().is_err());
    }
}

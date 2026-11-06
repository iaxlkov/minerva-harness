//! Cordis owns plugin lifetimes; the built-in manager publishes the roster service.
use crate::codex::{CodexPlugin, CodexService};
use crate::providers::{ProviderPlugin, ProviderService};
use crate::shell::{ShellPlugin, ShellService};
use cordis::event::{observer_sync, ListenerRegistrationError};
#[cfg(test)]
use cordis::Routing;
use cordis::{Context, Event, FiberHandle, FiberState, Plugin, PreparedPlugin, Service};
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

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    id: String,
    name: String,
    description: String,
    enabled: bool,
    can_toggle: bool,
    origin: String,
}

struct ManagerService {
    context: Context,
    example: Mutex<Option<FiberHandle>>,
    example_events: Arc<AtomicUsize>,
    external: Mutex<minerva_plugin_host::Host>,
    local: bool,
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
        let mut roster = vec![
            PluginInfo {
                id: "plugin-manager".into(),
                name: "Менеджер плагинов".into(),
                description: "Предоставляет сервис управления встроенными плагинами.".into(),
                enabled: true,
                can_toggle: false,
                origin: "builtin".into(),
            },
            PluginInfo {
                id: "providers".into(),
                name: "Провайдеры моделей".into(),
                description: "Конфигурация и подключение моделей.".into(),
                enabled: self.context.try_service::<ProviderService>().is_ok(),
                can_toggle: false,
                origin: "builtin".into(),
            },
            PluginInfo {
                id: "codex".into(),
                name: "Codex".into(),
                description: "Подключение Codex App Server.".into(),
                enabled: self.local && self.context.try_service::<CodexService>().is_ok(),
                can_toggle: false,
                origin: "builtin".into(),
            },
            PluginInfo {
                id: "shell".into(),
                name: "Командная оболочка".into(),
                description: "Запуск команд и ограничения вывода.".into(),
                enabled: self.local && self.context.try_service::<ShellService>().is_ok(),
                can_toggle: false,
                origin: "builtin".into(),
            },
            PluginInfo {
                id: "example".into(),
                name: "Тестовый плагин".into(),
                description: "Проверяет запуск, события и отключение в Cordis.".into(),
                enabled: example
                    .as_ref()
                    .is_some_and(|handle| handle.state() == FiberState::Active),
                can_toggle: true,
                origin: "builtin".into(),
            },
        ];
        roster.extend(
            self.external
                .lock()
                .await
                .roster()
                .into_iter()
                .map(|(m, enabled)| PluginInfo {
                    id: m.id.clone(),
                    name: m.name.unwrap_or(m.id),
                    description: m.description.unwrap_or_default(),
                    enabled,
                    can_toggle: true,
                    origin: "installed".into(),
                }),
        );
        roster
    }
    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<PluginInfo>, String> {
        if id != "example" && !["plugin-manager", "providers", "codex", "shell"].contains(&id) {
            self.external.lock().await.set_enabled(id, enabled).await?;
            return Ok(self.list().await);
        }
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
        self.external.lock().await.shutdown().await;
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
        Self::start_with_external_plugins(settings_path, true).await
    }
    pub async fn start_with_external_plugins(
        settings_path: PathBuf,
        local: bool,
    ) -> Result<Self, String> {
        Self::start_scoped(
            settings_path,
            local,
            crate::credential_store::CredentialScope::new("local")?,
        )
        .await
    }
    pub async fn start_scoped(
        settings_path: PathBuf,
        local: bool,
        scope: crate::credential_store::CredentialScope,
    ) -> Result<Self, String> {
        let mut external = minerva_plugin_host::Host::open(
            settings_path
                .parent()
                .ok_or("settings path has no parent")?
                .to_path_buf(),
            local,
        )?;
        external.restore().await?;
        // Read fallible configuration before publishing any lifecycle effects.
        let shell_service = Arc::new(ShellService::new(settings_path.clone())?);
        let provider_path = settings_path.with_file_name("providers.json");
        let provider_service = Arc::new(if local {
            ProviderService::new(provider_path)?
        } else {
            ProviderService::new_with_store(
                provider_path,
                scope,
                Arc::new(crate::credential_store::UnavailableCredentialStore),
                crate::network_policy::EndpointPolicy::Hosted,
            )?
        });
        let context = Context::new();
        let example_events = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(ManagerService {
            context: context.clone(),
            example: Mutex::new(None),
            example_events: example_events.clone(),
            external: Mutex::new(external),
            local,
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
                    CodexPlugin(Arc::new(if local {
                        CodexService::default()
                    } else {
                        CodexService::hosted_unavailable()
                    })),
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
    pub async fn plugin_call(
        &self,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.service()?
            .external
            .lock()
            .await
            .call(id, method, params)
            .await
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

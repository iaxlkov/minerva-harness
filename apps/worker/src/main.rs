use minerva_api::{worker_router, WorkerApiConfig};
use minerva_runtime::{Runtime, RuntimeConfig};
use minerva_worker_auth::AssertionKey;
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    assignment_id: Uuid,
    generation: u64,
    worker_id: Uuid,
    tenant_id: Uuid,
    workspace_id: Uuid,
    bind: SocketAddr,
    data_dir: PathBuf,
    config_dir: PathBuf,
    assertion_key_file: PathBuf,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Workspace Host failed: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(target_os = "linux") {
        return Err("hosted Workspace Host requires the Linux sandbox deployment".into());
    }
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--config") {
        return Err("usage: minerva-worker --config /run/minerva/worker.json".into());
    }
    let path = args.next().ok_or("worker config path required")?;
    if args.next().is_some() {
        return Err("unexpected worker argument".into());
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > 65536 {
        return Err("worker config too large".into());
    }
    let settings: Settings = serde_json::from_slice(&bytes)?;
    if !settings.assertion_key_file.is_absolute() {
        return Err("derived key file must be absolute".into());
    }
    let key = AssertionKey::new(std::fs::read(settings.assertion_key_file)?)?;
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: settings.tenant_id.to_string(),
            workspace_id: settings.workspace_id.to_string(),
            profile: "hosted-worker".into(),
            data_dir: settings.data_dir,
            config_dir: settings.config_dir,
        })
        .await?,
    );
    let router = worker_router(
        runtime.clone(),
        WorkerApiConfig {
            assignment_id: settings.assignment_id,
            generation: settings.generation,
            worker_id: settings.worker_id,
            tenant_id: settings.tenant_id,
            workspace_id: settings.workspace_id,
            assertion_key: key,
        },
    )
    .map_err(std::io::Error::other)?;
    let listener = tokio::net::TcpListener::bind(settings.bind).await?;
    let shutdown_runtime = runtime.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                if let Ok(mut terminate) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            let _ = shutdown_runtime.shutdown().await;
        })
        .await?;
    runtime.shutdown().await?;
    Ok(())
}

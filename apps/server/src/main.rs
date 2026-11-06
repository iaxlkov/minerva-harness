use minerva_api::{absolute_path, local_router, LocalApiConfig};
use minerva_runtime::{Runtime, RuntimeConfig};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Minerva: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut bind: SocketAddr = "127.0.0.1:3080".parse()?;
    let mut data_dir = PathBuf::from(".minerva/data");
    let mut config_dir = PathBuf::from(".minerva/config");
    let mut web_dist = PathBuf::from("dist");
    let mut dev_origin = None;
    let mut install_plugin: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install-plugin" => {
                install_plugin = Some(
                    args.next()
                        .ok_or("--install-plugin needs a built package directory")?
                        .into(),
                )
            }
            "--bind" => bind = args.next().ok_or("--bind needs an address")?.parse()?,
            "--data-dir" => data_dir = args.next().ok_or("--data-dir needs a path")?.into(),
            "--config-dir" => config_dir = args.next().ok_or("--config-dir needs a path")?.into(),
            "--web-dist" => web_dist = args.next().ok_or("--web-dist needs a path")?.into(),
            "--dev-origin" => dev_origin = Some(args.next().ok_or("--dev-origin needs an origin")?),
            "--help" => {
                println!("minerva-server [--bind 127.0.0.1:3080] [--data-dir PATH] [--config-dir PATH] [--web-dist dist] [--dev-origin http://127.0.0.1:1420]\nLocal web Host. External network binding requires the hosted deployment.");
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    if !bind.ip().is_loopback() {
        return Err("Local Host can bind only to loopback; use the authenticated hosted profile for external access".into());
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let mut config =
        LocalApiConfig::new(&origin, absolute_path(web_dist)?).map_err(std::io::Error::other)?;
    if let Some(ref dev) = dev_origin {
        config
            .allow_dev_origin(dev)
            .map_err(std::io::Error::other)?;
    }
    let launch_origin = dev_origin.as_deref().unwrap_or(&origin);
    let launch_url = format!("{launch_origin}/#minerva-token={}", config.launch_token);
    if let Some(package) = install_plugin {
        let id = minerva_plugin_host::install_local(
            &absolute_path(package)?,
            &absolute_path(config_dir.clone())?,
        )
        .map_err(std::io::Error::other)?;
        println!("Installed plugin {id}; enable it in Plugins.");
    }
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "local".into(),
            workspace_id: "default".into(),
            profile: "local".into(),
            data_dir: absolute_path(data_dir)?,
            config_dir: absolute_path(config_dir)?,
        })
        .await?,
    );
    let router = local_router(runtime.clone(), config);
    // One-time launch credential is displayed only at startup, never in HTTP URLs/logs.
    println!("Open Minerva (link valid for five minutes):\n{launch_url}");
    let shutdown_runtime = runtime.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_runtime.shutdown().await;
        })
        .await?;
    runtime.shutdown().await?;
    Ok(())
}

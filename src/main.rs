use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::{signal, sync::mpsc};
use tokio_util::sync::CancellationToken;

use npm_docker_sync::{
    config::{self, NpmCredential},
    docker::{DockerClient, labels::Defaults as LabelDefaults, watcher::Watcher},
    intent::Intent,
    metrics,
    npm::NpmClient,
    npm::auth::Credential,
    reconciler::Reconciler,
    telemetry,
    writer::{NpmWriter, WriterConfig},
};

#[derive(Parser)]
#[command(name = "npm-docker-sync", version, about)]
struct Cli {
    #[arg(long, default_value = "/etc/npm-docker-sync/config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let resolved = config::load(&cli.config).context("load config")?;
    telemetry::init(&resolved.config.logging)
        .map_err(|e| anyhow::anyhow!("init telemetry: {e}"))?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    let _metrics_guard = metrics::init(&resolved.config.metrics)
        .map_err(|e| anyhow::anyhow!("init metrics: {e}"))?;
    if let Some(handle) = &_metrics_guard {
        tracing::info!(bind = %handle.bound_addr(), "metrics endpoint listening");
    }

    let cred = match &resolved.npm_credential {
        NpmCredential::EmailPassword { email, password } => Credential::EmailPassword {
            email: email.clone(),
            password: password.clone(),
        },
        NpmCredential::Token(t) => Credential::Token(t.clone()),
    };
    let npm = NpmClient::connect(resolved.config.npm.url.clone(), cred)
        .await
        .context("connect to NPM")?;
    let docker =
        DockerClient::connect(&resolved.config.docker.socket).context("connect to Docker")?;

    let defaults = LabelDefaults {
        scheme: resolved.config.defaults.scheme,
        ssl: resolved.config.defaults.ssl,
        websockets: resolved.config.defaults.websockets,
        block_exploits: resolved.config.defaults.block_exploits,
    };
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel::<Intent>(256);

    let writer_cfg = WriterConfig {
        forward_strategy: resolved.config.forward_host.strategy,
        host_address: resolved.config.forward_host.host_address.clone(),
        network: resolved.config.forward_host.network.clone(),
        letsencrypt_email: resolved
            .config
            .npm
            .letsencrypt_email
            .clone()
            .unwrap_or_default(),
        cleanup_on_remove: resolved.config.cleanup.on_remove,
        cf_global: resolved.cloudflare_global.clone(),
        cf_per_domain: resolved.cloudflare_per_domain.clone(),
    };
    let writer_handle =
        tokio::spawn(NpmWriter::new(npm.clone_for_tasks(), writer_cfg, rx, cancel.clone()).run());

    let recon = Reconciler {
        docker: docker.clone_for_tasks(),
        npm: npm.clone_for_tasks(),
        defaults,
        interval: Duration::from_secs(resolved.config.reconciler.interval_seconds),
        tx: tx.clone(),
        cancel: cancel.clone(),
    };
    let recon_handle = tokio::spawn(recon.run());

    let watcher = Watcher::new(docker, defaults, tx, cancel.clone());
    let watcher_handle = tokio::spawn(watcher.run());

    shutdown_signal().await;
    tracing::info!("shutdown requested");
    cancel.cancel();
    let join_fut = async {
        let (w, r, wa) = tokio::join!(writer_handle, recon_handle, watcher_handle);
        (w.is_err(), r.is_err(), wa.is_err())
    };
    match tokio::time::timeout(Duration::from_secs(10), join_fut).await {
        Ok((we, re, wae)) => {
            if we || re || wae {
                tracing::warn!(
                    writer_errored = we,
                    reconciler_errored = re,
                    watcher_errored = wae,
                    "one or more tasks returned an error on shutdown"
                );
            }
        }
        Err(_) => tracing::error!("shutdown timed out after 10s — tasks still running"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        let _ = sig.recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

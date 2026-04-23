pub mod plan;

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::docker::{DockerClient, labels};
use crate::intent::Intent;
use crate::npm::NpmClient;
use crate::writer::retry_npm::with_retry;

pub struct Reconciler {
    pub docker: DockerClient,
    pub npm: NpmClient,
    pub defaults: labels::Defaults,
    pub interval: Duration,
    pub tx: mpsc::Sender<Intent>,
    pub cancel: CancellationToken,
}

impl Reconciler {
    pub async fn run(self) {
        // One immediate sweep at startup.
        if let Err(e) = self.sweep().await {
            tracing::warn!(error = %e, "initial reconciliation sweep failed");
        }
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(self.interval) => {
                    if let Err(e) = self.sweep().await {
                        tracing::warn!(error = %e, "reconciliation sweep failed");
                    }
                }
            }
        }
    }

    async fn sweep(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let summaries = self.docker.list_labeled().await?;
        let mut specs = Vec::new();
        for s in summaries {
            let Some(id) = s.id.clone() else { continue };
            let insp = match self.docker.inspect(&id).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Some(config) = insp.config else { continue };
            let raw = config.labels.unwrap_or_default();
            let labels_map: BTreeMap<String, String> = raw.into_iter().collect();
            let name = insp
                .name
                .as_deref()
                .unwrap_or(&id)
                .trim_start_matches('/')
                .to_string();
            let mut ports: Vec<u16> = config
                .exposed_ports
                .unwrap_or_default()
                .iter()
                .filter_map(|p| p.split('/').next().and_then(|n| n.parse::<u16>().ok()))
                .collect();
            ports.sort_unstable();
            ports.dedup();
            let mut aliases = BTreeMap::new();
            if let Some(ns) = insp
                .network_settings
                .as_ref()
                .and_then(|n| n.networks.clone())
            {
                for (net, net_cfg) in ns {
                    if let Some(ip) = net_cfg.ip_address {
                        if !ip.is_empty() {
                            aliases.insert(format!("{net}:ip"), ip);
                        }
                    }
                    if let Some(ref ar) = net_cfg.aliases {
                        for a in ar {
                            aliases.insert(format!("{net}:alias"), a.clone());
                        }
                    }
                }
            }
            let facts = labels::ContainerFacts {
                id: &id,
                name: &name,
                labels: &labels_map,
                exposed_ports: &ports,
                network_aliases: aliases,
            };
            if let Ok(spec) = labels::parse(facts, &self.defaults) {
                specs.push(spec);
            }
        }
        let npm = &self.npm;
        let hosts = with_retry(npm, || npm.list_proxy_hosts()).await?;
        for intent in plan::plan(&specs, &hosts) {
            let _ = self.tx.send(intent).await;
        }
        Ok(())
    }
}

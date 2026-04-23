use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::docker::DockerClient;
use crate::docker::labels::{self, ContainerFacts, Defaults};
use crate::docker::spec::ContainerSpec;
use crate::intent::Intent;

pub struct Watcher {
    docker: DockerClient,
    defaults: Defaults,
    tx: mpsc::Sender<Intent>,
    cancel: CancellationToken,
}

impl Watcher {
    pub fn new(
        docker: DockerClient,
        defaults: Defaults,
        tx: mpsc::Sender<Intent>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            docker,
            defaults,
            tx,
            cancel,
        }
    }

    pub async fn run(self) {
        let mut backoff = Duration::from_secs(1);
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            match self.run_once().await {
                Ok(()) => backoff = Duration::from_secs(1),
                Err(e) => {
                    tracing::warn!(error = %e, "watcher stream failed, reconnecting");
                    tokio::select! {
                        _ = self.cancel.cancelled() => return,
                        _ = tokio::time::sleep(backoff) => {}
                    }
                    backoff = std::cmp::min(backoff * 4, Duration::from_secs(60));
                }
            }
        }
    }

    async fn run_once(&self) -> Result<(), bollard::errors::Error> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("type".to_string(), vec!["container".to_string()]);
        filters.insert(
            "event".to_string(),
            vec![
                "start".to_string(),
                "die".to_string(),
                "destroy".to_string(),
            ],
        );

        let options = bollard::query_parameters::EventsOptionsBuilder::default()
            .filters(&filters)
            .build();

        let mut stream = self.docker.handle().events(Some(options));

        while let Some(ev) = stream.next().await {
            let ev = ev?;
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            let Some(actor) = ev.actor else { continue };
            let Some(id) = actor.id else { continue };
            if id.is_empty() {
                continue;
            }
            let action = ev.action.as_deref().unwrap_or("");
            match action {
                "start" => {
                    if let Some(spec) = self.build_spec(&id).await {
                        let _ = self
                            .tx
                            .send(Intent::Upsert {
                                container_id: id,
                                spec,
                            })
                            .await;
                    }
                }
                "die" | "destroy" => {
                    let _ = self.tx.send(Intent::Remove { container_id: id }).await;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn build_spec(&self, id: &str) -> Option<ContainerSpec> {
        let insp = match self.docker.inspect(id).await {
            Ok(v) => v,
            Err(_) => return None,
        };
        let config = insp.config?;
        let raw_labels = config.labels.unwrap_or_default();
        let labels_map: BTreeMap<String, String> = raw_labels.into_iter().collect();
        let name = insp
            .name
            .as_deref()
            .unwrap_or(id)
            .trim_start_matches('/')
            .to_string();
        // exposed_ports is Vec<String> in bollard 0.20, each entry is "80/tcp" format
        let mut exposed_ports: Vec<u16> = config
            .exposed_ports
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p.split('/').next().and_then(|n| n.parse::<u16>().ok()))
            .collect();
        exposed_ports.sort_unstable();
        exposed_ports.dedup();
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
        let facts = ContainerFacts {
            id,
            name: &name,
            labels: &labels_map,
            exposed_ports: &exposed_ports,
            network_aliases: aliases,
        };
        match labels::parse(facts, &self.defaults) {
            Ok(s) => Some(s),
            Err(labels::LabelError::NotLabeled) => None,
            Err(e) => {
                tracing::warn!(error = %e, container = %id, "skip: bad label");
                None
            }
        }
    }
}

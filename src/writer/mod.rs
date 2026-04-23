pub mod diff;
pub mod retry;
pub(crate) mod retry_npm;

use std::collections::{BTreeMap, HashMap};

use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cloudflare::TokenResolver;
use crate::config::ForwardStrategy;
use crate::docker::spec::{ContainerSpec, Scheme};
use crate::intent::Intent;
use crate::npm::NpmClient;
use crate::npm::certificates::LetsEncryptRequest;
use crate::npm::meta::{self, OwnershipMarker};
use crate::npm::types::{CreateProxyHost, UpdateProxyHost};
use crate::writer::diff::{DesiredProxyHost, diff_spec};
use crate::writer::retry_npm::with_retry;

pub struct WriterConfig {
    pub forward_strategy: ForwardStrategy,
    pub host_address: Option<String>,
    pub network: Option<String>,
    pub letsencrypt_email: String,
    pub cleanup_on_remove: bool,
    pub cf_global: Option<String>,
    pub cf_per_domain: BTreeMap<String, String>,
}

pub struct NpmWriter {
    npm: NpmClient,
    cfg: WriterConfig,
    rx: mpsc::Receiver<Intent>,
    cancel: CancellationToken,
    cache: HashMap<String, u64>, // container_id -> proxy_host_id
    cf: TokenResolver,
}

impl NpmWriter {
    pub fn new(
        npm: NpmClient,
        cfg: WriterConfig,
        rx: mpsc::Receiver<Intent>,
        cancel: CancellationToken,
    ) -> Self {
        let cf = TokenResolver::new(cfg.cf_global.clone(), cfg.cf_per_domain.clone());
        Self {
            npm,
            cfg,
            rx,
            cancel,
            cache: HashMap::new(),
            cf,
        }
    }

    pub async fn run(mut self) {
        // Seed cache from NPM on startup.
        let npm = &self.npm;
        match with_retry(npm, "list", || npm.list_proxy_hosts()).await {
            Ok(hosts) => {
                for h in hosts {
                    if let Some(m) = meta::decode(&h.meta) {
                        self.cache.insert(m.container_id, h.id);
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "initial cache seed failed"),
        }
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                msg = self.rx.recv() => {
                    let Some(intent) = msg else { return };
                    if let Err(e) = self.handle(intent).await {
                        tracing::error!(error = %e, "intent failed");
                    }
                }
            }
        }
    }

    async fn handle(
        &mut self,
        intent: Intent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match intent {
            Intent::Upsert { container_id, spec } => self.upsert(&container_id, &spec).await,
            Intent::Remove { container_id } => self.remove(&container_id).await,
        }
    }

    fn forward_host_for(&self, spec: &ContainerSpec) -> String {
        match self.cfg.forward_strategy {
            ForwardStrategy::ContainerName => spec.name.clone(),
            ForwardStrategy::ContainerIp => {
                let key = self.cfg.network.as_ref().map(|n| format!("{n}:ip"));
                key.and_then(|k| spec.network_aliases.get(&k).cloned())
                    .unwrap_or_else(|| spec.name.clone())
            }
            ForwardStrategy::HostPort => self
                .cfg
                .host_address
                .clone()
                .unwrap_or_else(|| "host.docker.internal".into()),
        }
    }

    async fn upsert(
        &mut self,
        container_id: &str,
        spec: &ContainerSpec,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let forward_host = self.forward_host_for(spec);
        let npm = &self.npm;
        let hosts = with_retry(npm, "list", || npm.list_proxy_hosts()).await?;
        let existing = hosts
            .iter()
            .find(|h| h.domain_names.iter().any(|d| d == &spec.url));

        match existing {
            Some(h) => {
                let marker = meta::decode(&h.meta);
                if marker.as_ref().map(|m| m.container_id.as_str()) != Some(container_id) {
                    tracing::error!(
                        url = %spec.url,
                        host_id = h.id,
                        "conflict: host exists for another owner, skipping"
                    );
                    return Ok(());
                }
                if let Some(patch) = diff_spec(
                    h,
                    &DesiredProxyHost {
                        spec,
                        forward_host: &forward_host,
                    },
                ) {
                    let host_id = h.id;
                    let npm = &self.npm;
                    with_retry(npm, "update", || npm.update_proxy_host(host_id, &patch)).await?;
                }
                self.cache.insert(container_id.to_string(), h.id);
            }
            None => {
                let marker = OwnershipMarker {
                    version: 1,
                    container_id: container_id.to_string(),
                    container_name: spec.name.clone(),
                    managed_at: OffsetDateTime::now_utc(),
                };
                let meta_v = meta::encode(&marker, None);
                let body = CreateProxyHost {
                    domain_names: vec![spec.url.clone()],
                    forward_scheme: match spec.scheme {
                        Scheme::Http => "http".into(),
                        Scheme::Https => "https".into(),
                    },
                    forward_host,
                    forward_port: spec.port,
                    allow_websocket_upgrade: spec.websockets,
                    block_exploits: spec.block_exploits,
                    caching_enabled: false,
                    meta: meta_v,
                    certificate_id: 0,
                    ssl_forced: false,
                    access_list_id: 0,
                };
                let npm = &self.npm;
                let created = with_retry(npm, "create", || npm.create_proxy_host(&body)).await?;
                self.cache.insert(container_id.to_string(), created.id);

                if spec.ssl {
                    let token = match self.cf.for_domain(&spec.url) {
                        Ok(t) => t.to_string(),
                        Err(e) => {
                            tracing::error!(error = %e, "no CF token, skipping cert");
                            return Ok(());
                        }
                    };
                    let credentials = format!("dns_cloudflare_api_token = {token}\n");
                    let cert_req = LetsEncryptRequest {
                        domain: spec.url.clone(),
                        letsencrypt_email: self.cfg.letsencrypt_email.clone(),
                        dns_provider_credentials: credentials,
                    };
                    let npm = &self.npm;
                    let cert_id =
                        with_retry(npm, "cert", || npm.request_certificate(&cert_req)).await?;
                    let mut patch = serde_json::Map::new();
                    patch.insert("certificate_id".into(), serde_json::Value::from(cert_id));
                    patch.insert("ssl_forced".into(), serde_json::Value::from(true));
                    let patch = UpdateProxyHost(patch);
                    let npm = &self.npm;
                    with_retry(npm, "update", || npm.update_proxy_host(created.id, &patch)).await?;
                }
            }
        }
        Ok(())
    }

    async fn remove(
        &mut self,
        container_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.cfg.cleanup_on_remove {
            return Ok(());
        }
        let host_id = match self.cache.get(container_id).copied() {
            Some(id) => id,
            None => return Ok(()),
        };
        let npm = &self.npm;
        let hosts = with_retry(npm, "list", || npm.list_proxy_hosts()).await?;
        let Some(host) = hosts.iter().find(|h| h.id == host_id) else {
            self.cache.remove(container_id);
            return Ok(());
        };
        let Some(marker) = meta::decode(&host.meta) else {
            tracing::warn!(host_id, "refuse to delete host without ownership marker");
            return Ok(());
        };
        if marker.container_id != container_id {
            tracing::warn!(host_id, "marker container_id mismatch; refusing to delete");
            return Ok(());
        }
        let npm = &self.npm;
        with_retry(npm, "delete", || npm.delete_proxy_host(host_id)).await?;
        self.cache.remove(container_id);
        Ok(())
    }
}

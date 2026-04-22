use crate::docker::spec::ContainerSpec;
use crate::intent::Intent;
use crate::npm::meta::decode;
use crate::npm::types::ProxyHost;
use std::collections::HashSet;

pub fn plan(containers: &[ContainerSpec], hosts: &[ProxyHost]) -> Vec<Intent> {
    let mut out = Vec::with_capacity(containers.len() + hosts.len());
    let alive: HashSet<&str> = containers.iter().map(|c| c.id.as_str()).collect();
    for spec in containers {
        out.push(Intent::Upsert {
            container_id: spec.id.clone(),
            spec: spec.clone(),
        });
    }
    for host in hosts {
        if let Some(marker) = decode(&host.meta) {
            if !alive.contains(marker.container_id.as_str()) {
                out.push(Intent::Remove {
                    container_id: marker.container_id,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::spec::ContainerSpec;
    use crate::npm::meta::{OwnershipMarker, encode};
    use serde_json::json;
    use time::macros::datetime;

    fn marker(container_id: &str) -> OwnershipMarker {
        OwnershipMarker {
            version: 1,
            container_id: container_id.into(),
            container_name: "x".into(),
            managed_at: datetime!(2026-04-21 0:00:00 UTC),
        }
    }

    fn host_with_marker(id: u64, domain: &str, container_id: &str) -> ProxyHost {
        ProxyHost {
            id,
            domain_names: vec![domain.into()],
            forward_scheme: "http".into(),
            forward_host: "x".into(),
            forward_port: 80,
            certificate_id: None,
            ssl_forced: false,
            caching_enabled: false,
            allow_websocket_upgrade: true,
            block_exploits: true,
            meta: encode(&marker(container_id), None),
        }
    }

    fn host_user_managed(id: u64, domain: &str) -> ProxyHost {
        ProxyHost {
            id,
            domain_names: vec![domain.into()],
            forward_scheme: "http".into(),
            forward_host: "x".into(),
            forward_port: 80,
            certificate_id: None,
            ssl_forced: false,
            caching_enabled: false,
            allow_websocket_upgrade: true,
            block_exploits: true,
            meta: json!({}),
        }
    }

    #[test]
    fn every_container_gets_upsert() {
        let cs = vec![ContainerSpec::stub("a", "a.example.com")];
        let out = plan(&cs, &[]);
        assert!(
            out.iter()
                .any(|i| matches!(i, Intent::Upsert { container_id, .. } if container_id == "a"))
        );
    }

    #[test]
    fn owned_host_without_container_gets_remove() {
        let hosts = vec![host_with_marker(1, "x.example.com", "gone")];
        let out = plan(&[], &hosts);
        assert!(
            out.iter()
                .any(|i| matches!(i, Intent::Remove { container_id } if container_id == "gone"))
        );
    }

    #[test]
    fn user_managed_hosts_ignored() {
        let hosts = vec![host_user_managed(1, "u.example.com")];
        let out = plan(&[], &hosts);
        assert!(out.is_empty());
    }

    #[test]
    fn existing_owned_host_with_live_container_no_remove() {
        let cs = vec![ContainerSpec::stub("a", "a.example.com")];
        let hosts = vec![host_with_marker(1, "a.example.com", "a")];
        let out = plan(&cs, &hosts);
        assert!(
            !out.iter()
                .any(|i| matches!(i, Intent::Remove { container_id } if container_id == "a"))
        );
    }
}

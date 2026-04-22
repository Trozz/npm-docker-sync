use crate::docker::spec::{ContainerSpec, Scheme};
use crate::npm::types::{ProxyHost, UpdateProxyHost};
use serde_json::Value;

pub struct DesiredProxyHost<'a> {
    pub spec: &'a ContainerSpec,
    pub forward_host: &'a str,
}

pub fn diff_spec(existing: &ProxyHost, desired: &DesiredProxyHost<'_>) -> Option<UpdateProxyHost> {
    let mut patch = serde_json::Map::new();
    let desired_scheme = match desired.spec.scheme {
        Scheme::Http => "http",
        Scheme::Https => "https",
    };
    if existing.forward_scheme != desired_scheme {
        patch.insert("forward_scheme".into(), Value::from(desired_scheme));
    }
    if existing.forward_host != desired.forward_host {
        patch.insert("forward_host".into(), Value::from(desired.forward_host));
    }
    if existing.forward_port != desired.spec.port {
        patch.insert("forward_port".into(), Value::from(desired.spec.port));
    }
    if existing.allow_websocket_upgrade != desired.spec.websockets {
        patch.insert(
            "allow_websocket_upgrade".into(),
            Value::from(desired.spec.websockets),
        );
    }
    if existing.block_exploits != desired.spec.block_exploits {
        patch.insert(
            "block_exploits".into(),
            Value::from(desired.spec.block_exploits),
        );
    }
    if existing.domain_names != vec![desired.spec.url.clone()] {
        patch.insert(
            "domain_names".into(),
            Value::from(vec![desired.spec.url.clone()]),
        );
    }
    if patch.is_empty() {
        None
    } else {
        Some(UpdateProxyHost(patch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn spec() -> ContainerSpec {
        ContainerSpec {
            id: "abc".into(),
            name: "myapp".into(),
            url: "app.example.com".into(),
            port: 3000,
            scheme: Scheme::Http,
            ssl: true,
            websockets: true,
            block_exploits: true,
            network_aliases: BTreeMap::new(),
        }
    }

    fn existing(port: u16) -> ProxyHost {
        ProxyHost {
            id: 1,
            domain_names: vec!["app.example.com".into()],
            forward_scheme: "http".into(),
            forward_host: "myapp".into(),
            forward_port: port,
            certificate_id: None,
            ssl_forced: false,
            caching_enabled: false,
            allow_websocket_upgrade: true,
            block_exploits: true,
            meta: json!({}),
        }
    }

    #[test]
    fn no_diff_when_equal() {
        let s = spec();
        let e = existing(3000);
        assert!(
            diff_spec(
                &e,
                &DesiredProxyHost {
                    spec: &s,
                    forward_host: "myapp"
                }
            )
            .is_none()
        );
    }

    #[test]
    fn diff_on_port_change() {
        let s = spec();
        let e = existing(9999);
        let patch = diff_spec(
            &e,
            &DesiredProxyHost {
                spec: &s,
                forward_host: "myapp",
            },
        )
        .unwrap();
        assert_eq!(patch.0.get("forward_port"), Some(&json!(3000)));
    }

    #[test]
    fn diff_on_scheme_change() {
        let mut s = spec();
        s.scheme = Scheme::Https;
        let e = existing(3000);
        let patch = diff_spec(
            &e,
            &DesiredProxyHost {
                spec: &s,
                forward_host: "myapp",
            },
        )
        .unwrap();
        assert_eq!(patch.0.get("forward_scheme"), Some(&json!("https")));
    }

    #[test]
    fn diff_on_forward_host_change() {
        let s = spec();
        let e = existing(3000);
        let patch = diff_spec(
            &e,
            &DesiredProxyHost {
                spec: &s,
                forward_host: "newname",
            },
        )
        .unwrap();
        assert_eq!(patch.0.get("forward_host"), Some(&json!("newname")));
    }

    #[test]
    fn diff_on_flag_change() {
        let mut s = spec();
        s.websockets = false;
        let e = existing(3000);
        let patch = diff_spec(
            &e,
            &DesiredProxyHost {
                spec: &s,
                forward_host: "myapp",
            },
        )
        .unwrap();
        assert_eq!(patch.0.get("allow_websocket_upgrade"), Some(&json!(false)));
    }
}

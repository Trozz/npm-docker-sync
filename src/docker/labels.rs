use crate::docker::spec::{ContainerSpec, Scheme};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LabelError {
    #[error("no nginx_proxy_url label present")]
    NotLabeled,
    #[error("invalid port value: {0}")]
    BadPort(String),
    #[error("invalid scheme: {0}")]
    BadScheme(String),
    #[error("invalid boolean for {key}: {value}")]
    BadBool { key: &'static str, value: String },
    #[error("container exposes no ports and no nginx_proxy_port label set")]
    NoPort,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Defaults {
    pub scheme: Scheme,
    pub ssl: bool,
    pub websockets: bool,
    pub block_exploits: bool,
}

pub struct ContainerFacts<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub labels: &'a BTreeMap<String, String>,
    pub exposed_ports: &'a [u16],
    pub network_aliases: BTreeMap<String, String>,
}

pub fn parse(facts: ContainerFacts<'_>, defaults: &Defaults) -> Result<ContainerSpec, LabelError> {
    let url = facts
        .labels
        .get("nginx_proxy_url")
        .ok_or(LabelError::NotLabeled)?
        .clone();
    let port = select_port(
        facts.exposed_ports,
        facts.labels.get("nginx_proxy_port").map(String::as_str),
    )?;
    let scheme = match facts.labels.get("nginx_proxy_scheme") {
        Some(s) => s
            .parse::<Scheme>()
            .map_err(|_| LabelError::BadScheme(s.clone()))?,
        None => defaults.scheme,
    };
    let ssl = match facts.labels.get("nginx_proxy_ssl") {
        Some(v) => parse_bool("nginx_proxy_ssl", v)?,
        None => defaults.ssl,
    };
    let websockets = match facts.labels.get("nginx_proxy_websockets") {
        Some(v) => parse_bool("nginx_proxy_websockets", v)?,
        None => defaults.websockets,
    };
    let block_exploits = match facts.labels.get("nginx_proxy_block_exploits") {
        Some(v) => parse_bool("nginx_proxy_block_exploits", v)?,
        None => defaults.block_exploits,
    };
    Ok(ContainerSpec {
        id: facts.id.to_string(),
        name: facts.name.to_string(),
        url,
        port,
        scheme,
        ssl,
        websockets,
        block_exploits,
        network_aliases: facts.network_aliases,
    })
}

pub fn select_port(exposed: &[u16], label: Option<&str>) -> Result<u16, LabelError> {
    if let Some(s) = label {
        return s
            .parse::<u16>()
            .map_err(|_| LabelError::BadPort(s.to_string()));
    }
    match exposed.len() {
        0 => Err(LabelError::NoPort),
        1 => Ok(exposed[0]),
        _ => {
            let mut sorted = exposed.to_vec();
            sorted.sort_unstable();
            tracing::warn!(ports = ?sorted, "multiple exposed ports, picking lowest");
            Ok(sorted[0])
        }
    }
}

fn parse_bool(key: &'static str, value: &str) -> Result<bool, LabelError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(LabelError::BadBool {
            key,
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn select_port_prefers_label() {
        assert_eq!(select_port(&[80, 3000], Some("3000")).unwrap(), 3000);
    }

    #[test]
    fn select_port_uses_sole_exposed_port() {
        assert_eq!(select_port(&[8080], None).unwrap(), 8080);
    }

    #[test]
    fn select_port_picks_lowest_with_multiple() {
        assert_eq!(select_port(&[3000, 80, 8080], None).unwrap(), 80);
    }

    #[test]
    fn select_port_rejects_non_numeric_label() {
        assert_eq!(
            select_port(&[80], Some("abc")),
            Err(LabelError::BadPort("abc".into()))
        );
    }

    #[test]
    fn select_port_errors_when_nothing_exposed() {
        assert_eq!(select_port(&[], None), Err(LabelError::NoPort));
    }

    #[test]
    fn parse_minimal_labels() {
        let labels = labels(&[("nginx_proxy_url", "app.example.com")]);
        let facts = ContainerFacts {
            id: "abc",
            name: "myapp",
            labels: &labels,
            exposed_ports: &[3000],
            network_aliases: BTreeMap::new(),
        };
        let spec = parse(
            facts,
            &Defaults {
                scheme: Scheme::Http,
                ssl: true,
                websockets: true,
                block_exploits: true,
            },
        )
        .unwrap();
        assert_eq!(spec.url, "app.example.com");
        assert_eq!(spec.port, 3000);
        assert_eq!(spec.scheme, Scheme::Http);
        assert!(spec.ssl);
    }

    #[test]
    fn parse_honors_all_label_overrides() {
        let labels = labels(&[
            ("nginx_proxy_url", "app.example.com"),
            ("nginx_proxy_port", "8080"),
            ("nginx_proxy_scheme", "https"),
            ("nginx_proxy_ssl", "false"),
            ("nginx_proxy_websockets", "false"),
            ("nginx_proxy_block_exploits", "false"),
        ]);
        let facts = ContainerFacts {
            id: "abc",
            name: "myapp",
            labels: &labels,
            exposed_ports: &[80],
            network_aliases: BTreeMap::new(),
        };
        let spec = parse(facts, &Defaults::default()).unwrap();
        assert_eq!(spec.port, 8080);
        assert_eq!(spec.scheme, Scheme::Https);
        assert!(!spec.ssl);
        assert!(!spec.websockets);
        assert!(!spec.block_exploits);
    }

    #[test]
    fn parse_errors_without_url_label() {
        let labels = labels(&[("other", "x")]);
        let facts = ContainerFacts {
            id: "abc",
            name: "myapp",
            labels: &labels,
            exposed_ports: &[80],
            network_aliases: BTreeMap::new(),
        };
        assert_eq!(
            parse(facts, &Defaults::default()).unwrap_err(),
            LabelError::NotLabeled
        );
    }

    #[test]
    fn parse_rejects_bad_bool() {
        let labels = labels(&[
            ("nginx_proxy_url", "app.example.com"),
            ("nginx_proxy_ssl", "yes"),
        ]);
        let facts = ContainerFacts {
            id: "abc",
            name: "myapp",
            labels: &labels,
            exposed_ports: &[80],
            network_aliases: BTreeMap::new(),
        };
        assert!(matches!(
            parse(facts, &Defaults::default()).unwrap_err(),
            LabelError::BadBool { .. }
        ));
    }
}

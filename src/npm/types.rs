use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyHost {
    pub id: u64,
    pub domain_names: Vec<String>,
    pub forward_scheme: String,
    pub forward_host: String,
    pub forward_port: u16,
    pub certificate_id: Option<u64>,
    pub ssl_forced: bool,
    pub caching_enabled: bool,
    pub allow_websocket_upgrade: bool,
    pub block_exploits: bool,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CreateProxyHost {
    pub domain_names: Vec<String>,
    pub forward_scheme: String,
    pub forward_host: String,
    pub forward_port: u16,
    pub allow_websocket_upgrade: bool,
    pub block_exploits: bool,
    pub caching_enabled: bool,
    pub meta: Value,
    pub certificate_id: u64,
    pub ssl_forced: bool,
    pub access_list_id: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateProxyHost(pub serde_json::Map<String, Value>);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CertificateRequest {
    pub domain_names: Vec<String>,
    pub meta: CertificateMeta,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CertificateMeta {
    pub letsencrypt_email: String,
    pub letsencrypt_agree: bool,
    pub dns_challenge: bool,
    pub dns_provider: String,
    pub dns_provider_credentials: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proxy_host_round_trips_minimal() {
        let raw = json!({
            "id": 1,
            "domain_names": ["x.example.com"],
            "forward_scheme": "http",
            "forward_host": "svc",
            "forward_port": 80,
            "certificate_id": null,
            "ssl_forced": false,
            "caching_enabled": false,
            "allow_websocket_upgrade": true,
            "block_exploits": true,
            "meta": {}
        });
        let ph: ProxyHost = serde_json::from_value(raw).unwrap();
        assert_eq!(ph.domain_names, vec!["x.example.com".to_string()]);
    }
}

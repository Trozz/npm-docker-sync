use npm_docker_sync::npm::proxy_hosts::ProxyHostClient;
use npm_docker_sync::npm::types::*;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn start() -> (MockServer, ProxyHostClient) {
    let server = MockServer::start().await;
    let c = ProxyHostClient::new(reqwest::Client::new(), server.uri(), "jwt".into());
    (server, c)
}

#[tokio::test]
async fn list_proxy_hosts_parses_response() {
    let (server, c) = start().await;
    Mock::given(method("GET"))
        .and(path("/api/nginx/proxy-hosts"))
        .and(header("authorization", "Bearer jwt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
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
        }])))
        .mount(&server)
        .await;

    let hosts = c.list().await.unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].domain_names, vec!["x.example.com"]);
}

#[tokio::test]
async fn create_sends_meta_and_parses_id() {
    let (server, c) = start().await;
    Mock::given(method("POST"))
        .and(path("/api/nginx/proxy-hosts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 42,
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
        })))
        .mount(&server)
        .await;

    let req = CreateProxyHost {
        domain_names: vec!["x.example.com".into()],
        forward_scheme: "http".into(),
        forward_host: "svc".into(),
        forward_port: 80,
        allow_websocket_upgrade: true,
        block_exploits: true,
        caching_enabled: false,
        meta: json!({ "npm_docker_sync": { "container_id": "abc" } }),
        certificate_id: 0,
        ssl_forced: false,
        access_list_id: 0,
    };
    assert_eq!(c.create(&req).await.unwrap().id, 42);
}

#[tokio::test]
async fn update_sends_patch() {
    let (server, c) = start().await;
    Mock::given(method("PUT"))
        .and(path("/api/nginx/proxy-hosts/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42,
            "domain_names": ["x.example.com"],
            "forward_scheme": "https",
            "forward_host": "svc",
            "forward_port": 443,
            "certificate_id": null,
            "ssl_forced": false,
            "caching_enabled": false,
            "allow_websocket_upgrade": true,
            "block_exploits": true,
            "meta": {}
        })))
        .mount(&server)
        .await;

    let mut patch = serde_json::Map::new();
    patch.insert("forward_port".into(), json!(443));
    let host = c.update(42, &UpdateProxyHost(patch)).await.unwrap();
    assert_eq!(host.forward_port, 443);
}

#[tokio::test]
async fn delete_returns_unit() {
    let (server, c) = start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/nginx/proxy-hosts/42"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    c.delete(42).await.unwrap();
}

use std::collections::BTreeMap;
use std::time::Duration;

use npm_docker_sync::config::ForwardStrategy;
use npm_docker_sync::docker::spec::{ContainerSpec, Scheme};
use npm_docker_sync::intent::Intent;
use npm_docker_sync::npm::NpmClient;
use npm_docker_sync::npm::auth::Credential;
use npm_docker_sync::writer::{NpmWriter, WriterConfig};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "jwt",
            "expires": "2099-01-01T00:00:00Z"
        })))
        .mount(server)
        .await;
}

fn sample_spec() -> ContainerSpec {
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

fn writer_config() -> WriterConfig {
    WriterConfig {
        forward_strategy: ForwardStrategy::ContainerName,
        host_address: None,
        network: None,
        letsencrypt_email: "ops@example.com".into(),
        cleanup_on_remove: true,
        cf_global: Some("tk".into()),
        cf_per_domain: BTreeMap::new(),
    }
}

#[tokio::test]
async fn upsert_creates_host_and_requests_certificate() {
    let server = MockServer::start().await;
    mount_login(&server).await;

    // Initial cache-seed list.
    Mock::given(method("GET"))
        .and(path("/api/nginx/proxy-hosts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/nginx/proxy-hosts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 1,
            "domain_names": ["app.example.com"],
            "forward_scheme": "http",
            "forward_host": "myapp",
            "forward_port": 3000,
            "certificate_id": null,
            "ssl_forced": false,
            "caching_enabled": false,
            "allow_websocket_upgrade": true,
            "block_exploits": true,
            "meta": {
                "npm_docker_sync": {
                    "version": 1,
                    "container_id": "abc",
                    "container_name": "myapp",
                    "managed_at": "2026-04-21T00:00:00Z"
                }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/nginx/certificates"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9 })))
        .expect(1)
        .mount(&server)
        .await;

    // Post-cert patch attaching certificate_id.
    Mock::given(method("PUT"))
        .and(path("/api/nginx/proxy-hosts/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1,
            "domain_names": ["app.example.com"],
            "forward_scheme": "http",
            "forward_host": "myapp",
            "forward_port": 3000,
            "certificate_id": 9,
            "ssl_forced": true,
            "caching_enabled": false,
            "allow_websocket_upgrade": true,
            "block_exploits": true,
            "meta": {}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let npm = NpmClient::connect(
        server.uri(),
        Credential::EmailPassword {
            email: "a".into(),
            password: "b".into(),
        },
    )
    .await
    .unwrap();
    let (tx, rx) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let handle = tokio::spawn(NpmWriter::new(npm, writer_config(), rx, cancel.clone()).run());

    tx.send(Intent::Upsert {
        container_id: "abc".into(),
        spec: sample_spec(),
    })
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    drop(tx);
    handle.await.unwrap();

    server.verify().await;
}

#[tokio::test]
async fn remove_respects_ownership_marker() {
    let server = MockServer::start().await;
    mount_login(&server).await;

    let host_json = json!([{
        "id": 1,
        "domain_names": ["app.example.com"],
        "forward_scheme": "http",
        "forward_host": "myapp",
        "forward_port": 3000,
        "certificate_id": null,
        "ssl_forced": false,
        "caching_enabled": false,
        "allow_websocket_upgrade": true,
        "block_exploits": true,
        "meta": {
            "npm_docker_sync": {
                "version": 1,
                "container_id": "abc",
                "container_name": "myapp",
                "managed_at": "2026-04-21T00:00:00Z"
            }
        }
    }]);

    Mock::given(method("GET"))
        .and(path("/api/nginx/proxy-hosts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(host_json))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/api/nginx/proxy-hosts/1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let npm = NpmClient::connect(
        server.uri(),
        Credential::EmailPassword {
            email: "a".into(),
            password: "b".into(),
        },
    )
    .await
    .unwrap();
    let (tx, rx) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let handle = tokio::spawn(NpmWriter::new(npm, writer_config(), rx, cancel.clone()).run());

    tx.send(Intent::Remove {
        container_id: "abc".into(),
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    drop(tx);
    handle.await.unwrap();

    server.verify().await;
}

//! End-to-end smoke test. Boots real Docker containers + the service binary as a subprocess.
//!
//! Run with: `cargo test --test e2e_smoke -- --ignored`
//!
//! Requires:
//! - Docker daemon running and accessible at /var/run/docker.sock (or DOCKER_HOST)
//! - Internet access so the test can pull images (jc21/nginx-proxy-manager, nginx:alpine)
//!
//! Note: Option A (subprocess) is used rather than Option B (containerised service) for speed.
//! The binary is located via the CARGO_BIN_EXE_npm-docker-sync env var that cargo sets
//! automatically for integration tests when the crate has a [[bin]] target.

#![cfg(test)]

use serde_json::Value;
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use testcontainers::{
    GenericImage,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires Docker and network access"]
async fn smoke_creates_and_removes_proxy_host() {
    let _ = tracing_subscriber::fmt::try_init();

    // --- 1. Start NPM --------------------------------------------------------
    // Expose port 81 (admin API) and let Docker pick a free host port.
    let npm = GenericImage::new("jc21/nginx-proxy-manager", "latest")
        .with_exposed_port(81.tcp())
        .with_exposed_port(80.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Started"))
        .start()
        .await
        .expect("start NPM container");

    let npm_host = npm.get_host().await.expect("npm host");
    let npm_port = npm.get_host_port_ipv4(81).await.expect("npm port 81");
    let npm_url = format!("http://{npm_host}:{npm_port}");

    // NPM needs a moment after "Started" before /api/tokens is live; retry login for ~60s.
    let http = reqwest::Client::new();
    let token = wait_for_npm_token(&http, &npm_url).await;

    // --- 2. Write a minimal config for the service ---------------------------
    let tempdir = TempDir::new().expect("tempdir");
    let config_path = tempdir.path().join("config.toml");
    let config = format!(
        r#"
[npm]
url = "{npm_url}"
email = "admin@example.com"
letsencrypt_email = "a@b"

[docker]
socket = "/var/run/docker.sock"

[forward_host]
strategy = "host_port"
host_address = "host.docker.internal"

[reconciler]
interval_seconds = 5

[cleanup]
on_remove = true

[defaults]
scheme = "http"
ssl = false
websockets = true
block_exploits = true

[logging]
level = "info"
format = "pretty"
"#
    );
    tokio::fs::write(&config_path, config)
        .await
        .expect("write config");

    // --- 3. Spawn the service binary as a subprocess -------------------------
    // CARGO_BIN_EXE_npm-docker-sync is set by cargo for integration tests
    // when the crate declares a bin target (implicit from src/main.rs).
    let bin = bin_path();
    let mut service = Command::new(&bin)
        .arg("--config")
        .arg(&config_path)
        .env("NPM_PASSWORD", "changeme")
        .env("RUST_LOG", "info,npm_docker_sync=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn service binary");

    // Give the service a moment to connect to NPM and run its initial reconciliation.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // --- 4. Start a labeled container ----------------------------------------
    let target_name = "npm-sync-smoke-target";
    // Use docker CLI here — simpler than negotiating with testcontainers
    // for the label semantics we need.
    let _ = Command::new("docker")
        .args(["rm", "-f", target_name])
        .output()
        .await;
    let run_status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            target_name,
            "-l",
            "nginx_proxy_url=smoke.localhost",
            "-l",
            "nginx_proxy_port=80",
            "nginx:alpine",
        ])
        .status()
        .await
        .expect("docker run target");
    assert!(run_status.success(), "docker run target container");

    // --- 5. Wait for the proxy host to appear in NPM -------------------------
    let host_id = wait_for_proxy_host(&http, &npm_url, &token, "smoke.localhost").await;
    eprintln!("proxy host created with id {host_id}");

    // --- 6. Stop the labeled container and wait for cleanup ------------------
    Command::new("docker")
        .args(["rm", "-f", target_name])
        .status()
        .await
        .expect("docker rm target");
    wait_for_proxy_host_gone(&http, &npm_url, &token, "smoke.localhost").await;

    // --- 7. Teardown ---------------------------------------------------------
    service.kill().await.expect("kill service");
    drop(npm); // testcontainers tears down the NPM container
}

fn bin_path() -> PathBuf {
    // CARGO_BIN_EXE_<name> is set for `cargo test` targets that declare a [[bin]].
    // The hyphen in the binary name is preserved verbatim in the env var name.
    let var = std::env::var("CARGO_BIN_EXE_npm-docker-sync")
        .expect("cargo should set CARGO_BIN_EXE_npm-docker-sync for integration tests");
    PathBuf::from(var)
}

async fn wait_for_npm_token(http: &reqwest::Client, npm_url: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let r = http
            .post(format!("{npm_url}/api/tokens"))
            .json(&serde_json::json!({ "identity": "admin@example.com", "secret": "changeme" }))
            .send()
            .await;
        if let Ok(resp) = r
            && resp.status().is_success()
            && let Ok(body) = resp.json::<Value>().await
            && let Some(tok) = body.get("token").and_then(Value::as_str)
        {
            return tok.to_string();
        }
        if Instant::now() >= deadline {
            panic!("npm /api/tokens never returned success within 60s");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_proxy_host(
    http: &reqwest::Client,
    npm_url: &str,
    token: &str,
    domain: &str,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(id) = find_proxy_host(http, npm_url, token, domain).await {
            return id;
        }
        if Instant::now() >= deadline {
            panic!("proxy host for {domain} never appeared within 30s");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_proxy_host_gone(
    http: &reqwest::Client,
    npm_url: &str,
    token: &str,
    domain: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if find_proxy_host(http, npm_url, token, domain)
            .await
            .is_none()
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("proxy host for {domain} was never removed within 30s");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn find_proxy_host(
    http: &reqwest::Client,
    npm_url: &str,
    token: &str,
    domain: &str,
) -> Option<u64> {
    let resp = http
        .get(format!("{npm_url}/api/nginx/proxy-hosts"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let list: Vec<Value> = resp.json().await.ok()?;
    for h in list {
        let names = h.get("domain_names").and_then(Value::as_array)?;
        if names.iter().any(|n| n.as_str() == Some(domain)) {
            return h.get("id").and_then(Value::as_u64);
        }
    }
    None
}

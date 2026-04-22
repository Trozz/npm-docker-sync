# NPM Docker Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust service that watches Docker containers for `nginx_proxy_url` labels and keeps an Nginx Proxy Manager (NPM) instance in sync, including Let's Encrypt SSL via Cloudflare DNS-01.

**Architecture:** Single tokio binary with three tasks communicating over one `mpsc` channel: a Docker event watcher, a periodic reconciler, and a single NPM writer actor. All NPM writes funnel through the actor so event-stream and reconciler work never races. Ownership is tracked via a marker in NPM's `meta` field — the service never touches hosts it doesn't own.

**Tech Stack:** Rust (edition 2024), tokio, bollard (Docker), reqwest (NPM HTTP), serde, toml, tracing, thiserror/anyhow, tokio-util. Dev: wiremock, testcontainers, pretty_assertions.

**Spec:** `docs/superpowers/specs/2026-04-21-npm-docker-integration-design.md`

---

## Before you begin

- **Branch.** All work happens on a new branch `feature/initial-implementation` off `develop`. The design spec lives on `feature/design-brainstorm` and will be merged into `develop` separately. From a clean working tree:
  ```bash
  git switch develop
  git switch -c feature/initial-implementation
  ```
- **Dependency versions.** User's CLAUDE.md mandates latest stable versions and up-to-date docs. Before scaffolding, resolve the current latest for each crate using context7 or `cargo search`. Versions shown in this plan are indicative — replace with whatever is current at implementation time:
  - `tokio`, `tokio-util`, `bollard`, `reqwest`, `serde`, `serde_json`, `toml`, `tracing`, `tracing-subscriber`, `thiserror`, `anyhow`, `async-trait`, `humantime-serde`, `url`
  - Dev-deps: `wiremock`, `testcontainers`, `pretty_assertions`, `tempfile`
- **Conventional commits.** Every commit message in this plan uses conventional-commit prefixes (`feat:`, `fix:`, `test:`, `chore:`, `docs:`, `refactor:`). Do not add `Co-Authored-By` trailers.
- **Staging.** Every `git add` in this plan names specific files. Do not use `git add -A` or `git add .`.
- **TDD.** Pure logic is written test-first: failing test → run (verify red) → minimal impl → run (verify green) → commit. I/O-heavy modules are tested via `wiremock` integration tests in `tests/`.

---

## File structure (target)

```
Cargo.toml
rust-toolchain.toml
rustfmt.toml
clippy.toml
README.md
.dockerignore
.github/workflows/ci.yml

src/
├── main.rs                     # binary entry, wire everything
├── lib.rs                      # pub modules for testing
├── intent.rs                   # Intent enum (channel message type)
├── cloudflare.rs               # TokenResolver (pure)
├── telemetry.rs                # tracing-subscriber init
│
├── config/
│   ├── mod.rs                  # Config struct, load(), validate()
│   └── secrets.rs              # env-var resolution
│
├── docker/
│   ├── mod.rs                  # DockerClient wrapper
│   ├── watcher.rs              # event-stream task
│   ├── labels.rs               # label parsing + port selection (pure)
│   └── spec.rs                 # ContainerSpec type
│
├── npm/
│   ├── mod.rs                  # NpmClient composition
│   ├── auth.rs                 # login + JWT refresh
│   ├── proxy_hosts.rs          # CRUD
│   ├── certificates.rs         # LE cert request
│   ├── meta.rs                 # ownership marker (pure)
│   └── types.rs                # ProxyHost, DesiredProxyHost, ...
│
├── writer/
│   ├── mod.rs                  # NpmWriter actor
│   ├── diff.rs                 # diff_spec (pure)
│   └── retry.rs                # exponential backoff
│
└── reconciler/
    ├── mod.rs                  # Reconciler task
    └── plan.rs                 # build intents (pure)

tests/
├── npm_mock.rs                 # wiremock integration
└── docker_e2e.rs               # testcontainers (ignored by default)

examples/config.toml
docker/Dockerfile
docker/docker-compose.example.yml
```

---

## Task 1: Project scaffolding

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.dockerignore`, `src/main.rs`, `src/lib.rs`

- [ ] **Step 1: Switch to implementation branch.**
  ```bash
  git switch develop
  git switch -c feature/initial-implementation
  ```

- [ ] **Step 2: Create `Cargo.toml`.** Use latest stable versions (verify via context7 / `cargo search` before committing):
  ```toml
  [package]
  name = "npm-docker-sync"
  version = "0.1.0"
  edition = "2024"
  rust-version = "1.85"
  description = "Sync Docker container labels into Nginx Proxy Manager"
  license = "MIT"

  [dependencies]
  tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "sync", "time", "fs"] }
  tokio-util = { version = "0.7", features = ["rt"] }
  bollard = "0.18"
  reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  toml = "0.8"
  tracing = "0.1"
  tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
  thiserror = "2"
  anyhow = "1"
  async-trait = "0.1"
  humantime-serde = "1"
  url = "2"
  futures-util = "0.3"
  time = { version = "0.3", features = ["serde", "formatting", "parsing"] }
  rand = "0.8"

  [dev-dependencies]
  wiremock = "0.6"
  testcontainers = "0.23"
  pretty_assertions = "1"
  tempfile = "3"

  [profile.release]
  lto = "thin"
  codegen-units = 1
  strip = "symbols"
  ```

- [ ] **Step 3: Pin toolchain.** `rust-toolchain.toml`:
  ```toml
  [toolchain]
  channel = "stable"
  components = ["rustfmt", "clippy"]
  ```

- [ ] **Step 4: Formatter/linter config.**
  `rustfmt.toml`:
  ```toml
  edition = "2024"
  max_width = 100
  ```
  `clippy.toml`:
  ```toml
  msrv = "1.85"
  ```

- [ ] **Step 5: `.dockerignore`.**
  ```
  target/
  .git/
  .github/
  docs/
  tests/
  examples/
  ```

- [ ] **Step 6: Stub `src/lib.rs`.**
  ```rust
  pub mod cloudflare;
  pub mod config;
  pub mod docker;
  pub mod intent;
  pub mod npm;
  pub mod reconciler;
  pub mod telemetry;
  pub mod writer;
  ```

- [ ] **Step 7: Stub each module as `mod.rs` with `// placeholder`** so the crate compiles:
  - `src/cloudflare.rs`
  - `src/config/mod.rs`, `src/config/secrets.rs`
  - `src/docker/mod.rs`, `src/docker/watcher.rs`, `src/docker/labels.rs`, `src/docker/spec.rs`
  - `src/intent.rs`
  - `src/npm/mod.rs`, `src/npm/auth.rs`, `src/npm/proxy_hosts.rs`, `src/npm/certificates.rs`, `src/npm/meta.rs`, `src/npm/types.rs`
  - `src/reconciler/mod.rs`, `src/reconciler/plan.rs`
  - `src/telemetry.rs`
  - `src/writer/mod.rs`, `src/writer/diff.rs`, `src/writer/retry.rs`

  Each file needs `// placeholder` plus the submodule declarations in `mod.rs` files:
  ```rust
  // src/config/mod.rs
  pub mod secrets;
  ```
  Repeat the pattern for `docker/mod.rs`, `npm/mod.rs`, `reconciler/mod.rs`, `writer/mod.rs`.

- [ ] **Step 8: Stub `src/main.rs`.**
  ```rust
  fn main() {
      println!("npm-docker-sync");
  }
  ```

- [ ] **Step 9: Verify build.**
  ```bash
  cargo build
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```
  Expected: all three succeed.

- [ ] **Step 10: Commit.**
  ```bash
  git add Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml clippy.toml .dockerignore src
  git commit -m "chore: scaffold Rust binary crate"
  ```

---

## Task 2: `Intent` enum (channel message type)

**Files:**
- Modify: `src/intent.rs`

- [ ] **Step 1: Write failing test.** Replace `src/intent.rs` with:
  ```rust
  use crate::docker::spec::ContainerSpec;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum Intent {
      Upsert { container_id: String, spec: ContainerSpec },
      Remove { container_id: String },
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::docker::spec::ContainerSpec;

      #[test]
      fn remove_intent_carries_container_id() {
          let i = Intent::Remove { container_id: "abc".into() };
          match i {
              Intent::Remove { container_id } => assert_eq!(container_id, "abc"),
              _ => panic!("wrong variant"),
          }
      }

      #[test]
      fn upsert_intent_carries_spec() {
          let spec = ContainerSpec::stub("abc", "app.example.com");
          let i = Intent::Upsert { container_id: "abc".into(), spec: spec.clone() };
          match i {
              Intent::Upsert { container_id, spec: got } => {
                  assert_eq!(container_id, "abc");
                  assert_eq!(got, spec);
              }
              _ => panic!("wrong variant"),
          }
      }
  }
  ```

- [ ] **Step 2: Run.** `cargo test --lib intent` — expect failure (`ContainerSpec` doesn't exist yet).

- [ ] **Step 3: Implement `ContainerSpec` stub** so the test compiles. In `src/docker/spec.rs`:
  ```rust
  use std::collections::BTreeMap;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ContainerSpec {
      pub id: String,
      pub name: String,
      pub url: String,
      pub port: u16,
      pub scheme: Scheme,
      pub ssl: bool,
      pub websockets: bool,
      pub block_exploits: bool,
      pub network_aliases: BTreeMap<String, String>, // network_name -> alias/ip
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Scheme { Http, Https }

  impl ContainerSpec {
      #[cfg(test)]
      pub fn stub(id: &str, url: &str) -> Self {
          Self {
              id: id.into(),
              name: "stub".into(),
              url: url.into(),
              port: 80,
              scheme: Scheme::Http,
              ssl: true,
              websockets: true,
              block_exploits: true,
              network_aliases: BTreeMap::new(),
          }
      }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib` — both tests pass.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/intent.rs src/docker/spec.rs
  git commit -m "feat: add Intent enum and ContainerSpec"
  ```

---

## Task 3: Label parsing and port selection (pure, TDD)

**Files:**
- Modify: `src/docker/labels.rs`, `src/docker/spec.rs` (add `Scheme` FromStr)

- [ ] **Step 1: Write failing tests.** Replace `src/docker/labels.rs`:
  ```rust
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

  #[derive(Debug, Default)]
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
      todo!()
  }

  pub fn select_port(exposed: &[u16], label: Option<&str>) -> Result<u16, LabelError> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use pretty_assertions::assert_eq;

      fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
          pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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
          assert_eq!(select_port(&[80], Some("abc")), Err(LabelError::BadPort("abc".into())));
      }

      #[test]
      fn select_port_errors_when_nothing_exposed() {
          assert_eq!(select_port(&[], None), Err(LabelError::NoPort));
      }

      #[test]
      fn parse_minimal_labels() {
          let labels = labels(&[("nginx_proxy_url", "app.example.com")]);
          let facts = ContainerFacts {
              id: "abc", name: "myapp", labels: &labels,
              exposed_ports: &[3000], network_aliases: BTreeMap::new(),
          };
          let spec = parse(facts, &Defaults { scheme: Scheme::Http, ssl: true, websockets: true, block_exploits: true }).unwrap();
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
              id: "abc", name: "myapp", labels: &labels,
              exposed_ports: &[80], network_aliases: BTreeMap::new(),
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
              id: "abc", name: "myapp", labels: &labels,
              exposed_ports: &[80], network_aliases: BTreeMap::new(),
          };
          assert_eq!(parse(facts, &Defaults::default()).unwrap_err(), LabelError::NotLabeled);
      }

      #[test]
      fn parse_rejects_bad_bool() {
          let labels = labels(&[
              ("nginx_proxy_url", "app.example.com"),
              ("nginx_proxy_ssl", "yes"),
          ]);
          let facts = ContainerFacts {
              id: "abc", name: "myapp", labels: &labels,
              exposed_ports: &[80], network_aliases: BTreeMap::new(),
          };
          assert!(matches!(parse(facts, &Defaults::default()).unwrap_err(), LabelError::BadBool { .. }));
      }
  }
  ```

  Also add `Scheme` defaulting and parsing in `src/docker/spec.rs`:
  ```rust
  impl Default for Scheme {
      fn default() -> Self { Scheme::Http }
  }

  impl std::str::FromStr for Scheme {
      type Err = ();
      fn from_str(s: &str) -> Result<Self, ()> {
          match s.eq_ignore_ascii_case("https") {
              true => Ok(Scheme::Https),
              false if s.eq_ignore_ascii_case("http") => Ok(Scheme::Http),
              _ => Err(()),
          }
      }
  }
  ```

- [ ] **Step 2: Run tests — expect failures.** `cargo test --lib docker::labels`. All fail with `not yet implemented`.

- [ ] **Step 3: Implement.** Replace the `todo!()` bodies:
  ```rust
  pub fn select_port(exposed: &[u16], label: Option<&str>) -> Result<u16, LabelError> {
      if let Some(s) = label {
          return s.parse::<u16>().map_err(|_| LabelError::BadPort(s.to_string()));
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
          _ => Err(LabelError::BadBool { key, value: value.to_string() }),
      }
  }

  pub fn parse(facts: ContainerFacts<'_>, defaults: &Defaults) -> Result<ContainerSpec, LabelError> {
      let url = facts.labels.get("nginx_proxy_url").ok_or(LabelError::NotLabeled)?.clone();
      let port = select_port(facts.exposed_ports, facts.labels.get("nginx_proxy_port").map(String::as_str))?;
      let scheme = match facts.labels.get("nginx_proxy_scheme") {
          Some(s) => s.parse::<Scheme>().map_err(|_| LabelError::BadScheme(s.clone()))?,
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
          url, port, scheme, ssl, websockets, block_exploits,
          network_aliases: facts.network_aliases,
      })
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib docker::labels` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/docker/labels.rs src/docker/spec.rs
  git commit -m "feat: parse Docker labels and select proxy port"
  ```

---

## Task 4: Ownership marker (pure, TDD)

**Files:**
- Modify: `src/npm/meta.rs`

- [ ] **Step 1: Write failing tests.** Replace `src/npm/meta.rs`:
  ```rust
  use serde::{Deserialize, Serialize};
  use serde_json::{Map, Value};
  use time::OffsetDateTime;

  pub const MARKER_KEY: &str = "npm_docker_sync";

  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  pub struct OwnershipMarker {
      pub version: u32,
      pub container_id: String,
      pub container_name: String,
      #[serde(with = "time::serde::rfc3339")]
      pub managed_at: OffsetDateTime,
  }

  pub fn encode(marker: &OwnershipMarker, existing_meta: Option<&Value>) -> Value {
      todo!()
  }

  pub fn decode(meta: &Value) -> Option<OwnershipMarker> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use serde_json::json;
      use time::macros::datetime;

      fn sample() -> OwnershipMarker {
          OwnershipMarker {
              version: 1,
              container_id: "7b3f".into(),
              container_name: "myapp".into(),
              managed_at: datetime!(2026-04-21 12:34:56 UTC),
          }
      }

      #[test]
      fn encode_into_fresh_meta() {
          let v = encode(&sample(), None);
          let marker = v.get("npm_docker_sync").expect("marker key present");
          assert_eq!(marker["container_id"], "7b3f");
          assert_eq!(marker["version"], 1);
      }

      #[test]
      fn encode_preserves_existing_keys() {
          let existing = json!({ "user_notes": "do not touch" });
          let v = encode(&sample(), Some(&existing));
          assert_eq!(v["user_notes"], "do not touch");
          assert!(v.get("npm_docker_sync").is_some());
      }

      #[test]
      fn encode_overwrites_stale_marker() {
          let existing = json!({ "npm_docker_sync": { "container_id": "old" } });
          let v = encode(&sample(), Some(&existing));
          assert_eq!(v["npm_docker_sync"]["container_id"], "7b3f");
      }

      #[test]
      fn decode_none_when_key_missing() {
          assert!(decode(&json!({})).is_none());
      }

      #[test]
      fn decode_round_trips() {
          let v = encode(&sample(), None);
          let got = decode(&v).expect("decoded");
          assert_eq!(got, sample());
      }

      #[test]
      fn decode_none_on_malformed_marker() {
          let v = json!({ "npm_docker_sync": "nonsense" });
          assert!(decode(&v).is_none());
      }
  }
  ```

- [ ] **Step 2: Run — expect failures.** `cargo test --lib npm::meta`.

- [ ] **Step 3: Implement.**
  ```rust
  pub fn encode(marker: &OwnershipMarker, existing_meta: Option<&Value>) -> Value {
      let mut obj: Map<String, Value> = match existing_meta {
          Some(Value::Object(m)) => m.clone(),
          _ => Map::new(),
      };
      obj.insert(MARKER_KEY.to_string(), serde_json::to_value(marker).expect("serialize marker"));
      Value::Object(obj)
  }

  pub fn decode(meta: &Value) -> Option<OwnershipMarker> {
      let obj = meta.as_object()?;
      let raw = obj.get(MARKER_KEY)?;
      serde_json::from_value::<OwnershipMarker>(raw.clone()).ok()
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib npm::meta` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/npm/meta.rs
  git commit -m "feat: encode and decode NPM ownership markers"
  ```

---

## Task 5: NPM types (proxy host structures)

**Files:**
- Modify: `src/npm/types.rs`

- [ ] **Step 1: Write `types.rs`** covering the minimal surface the rest of the service uses. Keep field coverage to what we read or write; everything else stays in `serde_json::Value` to avoid brittle schema coupling:
  ```rust
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
      pub certificate_id: u64,   // 0 when ssl disabled
      pub ssl_forced: bool,
      pub access_list_id: u64,   // 0 = public
  }

  #[derive(Debug, Clone, Serialize, PartialEq, Eq)]
  pub struct UpdateProxyHost(pub serde_json::Map<String, Value>);

  #[derive(Debug, Clone, Serialize, PartialEq, Eq)]
  pub struct CertificateRequest {
      pub domain_names: Vec<String>,
      pub meta: CertificateMeta,
      pub provider: String,   // "letsencrypt"
  }

  #[derive(Debug, Clone, Serialize, PartialEq, Eq)]
  pub struct CertificateMeta {
      pub letsencrypt_email: String,
      pub letsencrypt_agree: bool,
      pub dns_challenge: bool,
      pub dns_provider: String,      // "cloudflare"
      pub dns_provider_credentials: String,
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use serde_json::json;

      #[test]
      fn proxy_host_round_trips_minimal() {
          let raw = json!({
              "id": 1, "domain_names": ["x.example.com"],
              "forward_scheme": "http", "forward_host": "svc", "forward_port": 80,
              "certificate_id": null, "ssl_forced": false, "caching_enabled": false,
              "allow_websocket_upgrade": true, "block_exploits": true, "meta": {}
          });
          let ph: ProxyHost = serde_json::from_value(raw.clone()).unwrap();
          assert_eq!(ph.domain_names, vec!["x.example.com".to_string()]);
      }
  }
  ```

- [ ] **Step 2: Run.** `cargo test --lib npm::types`.

- [ ] **Step 3: Commit.**
  ```bash
  git add src/npm/types.rs
  git commit -m "feat: add NPM proxy-host and certificate types"
  ```

---

## Task 6: Diff function (pure, TDD)

**Files:**
- Modify: `src/writer/diff.rs`

- [ ] **Step 1: Write failing tests.**
  ```rust
  use crate::docker::spec::{ContainerSpec, Scheme};
  use crate::npm::types::{ProxyHost, UpdateProxyHost};
  use serde_json::{json, Value};

  pub struct DesiredProxyHost<'a> {
      pub spec: &'a ContainerSpec,
      pub forward_host: &'a str,
  }

  pub fn diff_spec(existing: &ProxyHost, desired: &DesiredProxyHost<'_>) -> Option<UpdateProxyHost> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use pretty_assertions::assert_eq;
      use std::collections::BTreeMap;

      fn spec() -> ContainerSpec {
          ContainerSpec {
              id: "abc".into(), name: "myapp".into(), url: "app.example.com".into(),
              port: 3000, scheme: Scheme::Http, ssl: true,
              websockets: true, block_exploits: true,
              network_aliases: BTreeMap::new(),
          }
      }

      fn existing(port: u16) -> ProxyHost {
          ProxyHost {
              id: 1, domain_names: vec!["app.example.com".into()],
              forward_scheme: "http".into(), forward_host: "myapp".into(), forward_port: port,
              certificate_id: None, ssl_forced: false, caching_enabled: false,
              allow_websocket_upgrade: true, block_exploits: true, meta: json!({}),
          }
      }

      #[test]
      fn no_diff_when_equal() {
          let s = spec();
          let e = existing(3000);
          assert!(diff_spec(&e, &DesiredProxyHost { spec: &s, forward_host: "myapp" }).is_none());
      }

      #[test]
      fn diff_on_port_change() {
          let s = spec();
          let e = existing(9999);
          let patch = diff_spec(&e, &DesiredProxyHost { spec: &s, forward_host: "myapp" }).unwrap();
          assert_eq!(patch.0.get("forward_port"), Some(&json!(3000)));
      }

      #[test]
      fn diff_on_scheme_change() {
          let mut s = spec();
          s.scheme = Scheme::Https;
          let e = existing(3000);
          let patch = diff_spec(&e, &DesiredProxyHost { spec: &s, forward_host: "myapp" }).unwrap();
          assert_eq!(patch.0.get("forward_scheme"), Some(&json!("https")));
      }

      #[test]
      fn diff_on_forward_host_change() {
          let s = spec();
          let e = existing(3000);
          let patch = diff_spec(&e, &DesiredProxyHost { spec: &s, forward_host: "newname" }).unwrap();
          assert_eq!(patch.0.get("forward_host"), Some(&json!("newname")));
      }

      #[test]
      fn diff_on_flag_change() {
          let mut s = spec();
          s.websockets = false;
          let e = existing(3000);
          let patch = diff_spec(&e, &DesiredProxyHost { spec: &s, forward_host: "myapp" }).unwrap();
          assert_eq!(patch.0.get("allow_websocket_upgrade"), Some(&json!(false)));
      }
  }
  ```

- [ ] **Step 2: Run — expect failures.** `cargo test --lib writer::diff`.

- [ ] **Step 3: Implement.**
  ```rust
  pub fn diff_spec(existing: &ProxyHost, desired: &DesiredProxyHost<'_>) -> Option<UpdateProxyHost> {
      let mut patch = serde_json::Map::new();
      let desired_scheme = match desired.spec.scheme {
          Scheme::Http => "http", Scheme::Https => "https",
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
          patch.insert("allow_websocket_upgrade".into(), Value::from(desired.spec.websockets));
      }
      if existing.block_exploits != desired.spec.block_exploits {
          patch.insert("block_exploits".into(), Value::from(desired.spec.block_exploits));
      }
      if existing.domain_names != vec![desired.spec.url.clone()] {
          patch.insert("domain_names".into(), Value::from(vec![desired.spec.url.clone()]));
      }
      if patch.is_empty() { None } else { Some(UpdateProxyHost(patch)) }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib writer::diff` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/writer/diff.rs
  git commit -m "feat: diff desired proxy host against existing"
  ```

---

## Task 7: Cloudflare token resolver (pure, TDD)

**Files:**
- Modify: `src/cloudflare.rs`

- [ ] **Step 1: Write failing tests.**
  ```rust
  use std::collections::BTreeMap;

  #[derive(Debug, thiserror::Error, PartialEq, Eq)]
  pub enum TokenError {
      #[error("no Cloudflare token for domain {0} and no global fallback")]
      NotFound(String),
  }

  pub struct TokenResolver {
      global: Option<String>,
      per_domain: BTreeMap<String, String>, // domain -> token
  }

  impl TokenResolver {
      pub fn new(global: Option<String>, per_domain: BTreeMap<String, String>) -> Self {
          Self { global, per_domain }
      }

      pub fn for_domain(&self, domain: &str) -> Result<&str, TokenError> {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn falls_back_to_global() {
          let r = TokenResolver::new(Some("global".into()), BTreeMap::new());
          assert_eq!(r.for_domain("any.example.com").unwrap(), "global");
      }

      #[test]
      fn prefers_exact_domain_match() {
          let mut map = BTreeMap::new();
          map.insert("example.com".into(), "scoped".into());
          let r = TokenResolver::new(Some("global".into()), map);
          assert_eq!(r.for_domain("example.com").unwrap(), "scoped");
      }

      #[test]
      fn matches_parent_domain_suffix() {
          let mut map = BTreeMap::new();
          map.insert("example.com".into(), "scoped".into());
          let r = TokenResolver::new(None, map);
          assert_eq!(r.for_domain("app.example.com").unwrap(), "scoped");
      }

      #[test]
      fn errors_without_any_match() {
          let r = TokenResolver::new(None, BTreeMap::new());
          assert!(matches!(r.for_domain("x.com"), Err(TokenError::NotFound(_))));
      }
  }
  ```

- [ ] **Step 2: Run — expect failures.** `cargo test --lib cloudflare`.

- [ ] **Step 3: Implement.**
  ```rust
  impl TokenResolver {
      pub fn for_domain(&self, domain: &str) -> Result<&str, TokenError> {
          if let Some(t) = self.per_domain.get(domain) {
              return Ok(t);
          }
          // longest-suffix match
          let best = self.per_domain.iter()
              .filter(|(k, _)| domain.ends_with(&format!(".{k}")) || domain == *k)
              .max_by_key(|(k, _)| k.len())
              .map(|(_, v)| v.as_str());
          if let Some(t) = best { return Ok(t); }
          self.global.as_deref().ok_or_else(|| TokenError::NotFound(domain.to_string()))
      }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib cloudflare` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/cloudflare.rs
  git commit -m "feat: resolve Cloudflare tokens per-domain with fallback"
  ```

---

## Task 8: Reconciler planning (pure, TDD)

**Files:**
- Modify: `src/reconciler/plan.rs`

- [ ] **Step 1: Write failing tests.**
  ```rust
  use crate::docker::spec::ContainerSpec;
  use crate::intent::Intent;
  use crate::npm::meta::{decode, OwnershipMarker};
  use crate::npm::types::ProxyHost;
  use std::collections::HashSet;

  pub fn plan(containers: &[ContainerSpec], hosts: &[ProxyHost]) -> Vec<Intent> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::docker::spec::ContainerSpec;
      use crate::npm::meta::{encode, OwnershipMarker};
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
              id, domain_names: vec![domain.into()],
              forward_scheme: "http".into(), forward_host: "x".into(), forward_port: 80,
              certificate_id: None, ssl_forced: false, caching_enabled: false,
              allow_websocket_upgrade: true, block_exploits: true,
              meta: encode(&marker(container_id), None),
          }
      }

      fn host_user_managed(id: u64, domain: &str) -> ProxyHost {
          ProxyHost {
              id, domain_names: vec![domain.into()],
              forward_scheme: "http".into(), forward_host: "x".into(), forward_port: 80,
              certificate_id: None, ssl_forced: false, caching_enabled: false,
              allow_websocket_upgrade: true, block_exploits: true,
              meta: json!({}),
          }
      }

      #[test]
      fn every_container_gets_upsert() {
          let cs = vec![ContainerSpec::stub("a", "a.example.com")];
          let out = plan(&cs, &[]);
          assert!(out.iter().any(|i| matches!(i, Intent::Upsert { container_id, .. } if container_id == "a")));
      }

      #[test]
      fn owned_host_without_container_gets_remove() {
          let hosts = vec![host_with_marker(1, "x.example.com", "gone")];
          let out = plan(&[], &hosts);
          assert!(out.iter().any(|i| matches!(i, Intent::Remove { container_id } if container_id == "gone")));
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
          // one upsert (diff applies later in writer), zero removes for "a"
          assert!(!out.iter().any(|i| matches!(i, Intent::Remove { container_id } if container_id == "a")));
      }
  }
  ```

- [ ] **Step 2: Run — expect failures.** `cargo test --lib reconciler::plan`.

- [ ] **Step 3: Implement.**
  ```rust
  pub fn plan(containers: &[ContainerSpec], hosts: &[ProxyHost]) -> Vec<Intent> {
      let mut out = Vec::with_capacity(containers.len() + hosts.len());
      let alive: HashSet<&str> = containers.iter().map(|c| c.id.as_str()).collect();
      for spec in containers {
          out.push(Intent::Upsert { container_id: spec.id.clone(), spec: spec.clone() });
      }
      for host in hosts {
          if let Some(marker) = decode(&host.meta) {
              if !alive.contains(marker.container_id.as_str()) {
                  out.push(Intent::Remove { container_id: marker.container_id });
              }
          }
      }
      out
  }
  ```

- [ ] **Step 4: Run.** `cargo test --lib reconciler::plan` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/reconciler/plan.rs
  git commit -m "feat: compute reconciliation intents from container and host state"
  ```

---

## Task 9: Config loading with env-var secrets (TDD)

**Files:**
- Modify: `src/config/mod.rs`, `src/config/secrets.rs`

- [ ] **Step 1: Write `secrets.rs` helper.**
  ```rust
  use std::env;

  #[derive(Debug, thiserror::Error, PartialEq, Eq)]
  pub enum SecretError {
      #[error("required env var {0} is not set")]
      Missing(String),
  }

  pub fn require(key: &str) -> Result<String, SecretError> {
      env::var(key).map_err(|_| SecretError::Missing(key.to_string()))
  }

  pub fn optional(key: &str) -> Option<String> {
      env::var(key).ok().filter(|s| !s.is_empty())
  }
  ```

- [ ] **Step 2: Write `config/mod.rs` scaffolding + failing tests.**
  ```rust
  pub mod secrets;

  use self::secrets::SecretError;
  use crate::docker::spec::Scheme;
  use serde::Deserialize;
  use std::{collections::BTreeMap, path::Path};

  #[derive(Debug, thiserror::Error)]
  pub enum ConfigError {
      #[error("read config: {0}")]
      Io(#[from] std::io::Error),
      #[error("parse config: {0}")]
      Parse(#[from] toml::de::Error),
      #[error(transparent)]
      Secret(#[from] SecretError),
      #[error("validation: {0}")]
      Validation(String),
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct Config {
      pub npm: NpmConfig,
      #[serde(default)]
      pub docker: DockerConfig,
      #[serde(default)]
      pub forward_host: ForwardHostConfig,
      #[serde(default)]
      pub reconciler: ReconcilerConfig,
      #[serde(default)]
      pub cleanup: CleanupConfig,
      #[serde(default)]
      pub cloudflare: CloudflareConfig,
      #[serde(default)]
      pub defaults: Defaults,
      #[serde(default)]
      pub logging: LoggingConfig,
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct NpmConfig {
      pub url: String,
      pub email: Option<String>,
      pub token_env: Option<String>,
      pub letsencrypt_email: Option<String>,
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct DockerConfig { pub socket: String }
  impl Default for DockerConfig {
      fn default() -> Self { Self { socket: "/var/run/docker.sock".into() } }
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct ForwardHostConfig {
      pub strategy: ForwardStrategy,
      pub host_address: Option<String>,
      pub network: Option<String>,
  }
  impl Default for ForwardHostConfig {
      fn default() -> Self { Self { strategy: ForwardStrategy::ContainerName, host_address: None, network: None } }
  }

  #[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
  #[serde(rename_all = "snake_case")]
  pub enum ForwardStrategy { ContainerName, ContainerIp, HostPort }

  #[derive(Debug, Clone, Deserialize)]
  pub struct ReconcilerConfig { pub interval_seconds: u64 }
  impl Default for ReconcilerConfig { fn default() -> Self { Self { interval_seconds: 300 } } }

  #[derive(Debug, Clone, Deserialize)]
  pub struct CleanupConfig { pub on_remove: bool }
  impl Default for CleanupConfig { fn default() -> Self { Self { on_remove: true } } }

  #[derive(Debug, Clone, Default, Deserialize)]
  pub struct CloudflareConfig {
      #[serde(default)]
      pub domains: BTreeMap<String, String>, // domain -> env var name
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct Defaults {
      pub scheme: Scheme, pub ssl: bool, pub websockets: bool, pub block_exploits: bool,
  }
  impl Default for Defaults {
      fn default() -> Self { Self { scheme: Scheme::Http, ssl: true, websockets: true, block_exploits: true } }
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct LoggingConfig { pub level: String, pub format: LogFormat }
  impl Default for LoggingConfig {
      fn default() -> Self { Self { level: "info".into(), format: LogFormat::Json } }
  }
  #[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
  #[serde(rename_all = "lowercase")]
  pub enum LogFormat { Json, Pretty }

  /// Loaded secrets resolved from env vars, paired with the parsed Config.
  #[derive(Debug, Clone)]
  pub struct ResolvedConfig {
      pub config: Config,
      pub npm_credential: NpmCredential,
      pub cloudflare_global: Option<String>,
      pub cloudflare_per_domain: BTreeMap<String, String>, // domain -> token
  }

  #[derive(Debug, Clone)]
  pub enum NpmCredential {
      EmailPassword { email: String, password: String },
      Token(String),
  }

  pub fn load(path: &Path) -> Result<ResolvedConfig, ConfigError> {
      let text = std::fs::read_to_string(path)?;
      let config: Config = toml::from_str(&text)?;
      resolve_secrets(config)
  }

  pub fn resolve_secrets(config: Config) -> Result<ResolvedConfig, ConfigError> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use std::sync::Mutex;

      // env is global; serialize tests that poke it
      static ENV_LOCK: Mutex<()> = Mutex::new(());

      fn with_env<R>(pairs: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
          let _g = ENV_LOCK.lock().unwrap();
          let prior: Vec<(String, Option<String>)> = pairs.iter()
              .map(|(k, _)| (k.to_string(), std::env::var(*k).ok())).collect();
          for (k, v) in pairs { std::env::set_var(k, v); }
          let out = f();
          for (k, v) in &prior {
              match v { Some(x) => std::env::set_var(k, x), None => std::env::remove_var(k) }
          }
          out
      }

      fn base_config() -> Config {
          Config {
              npm: NpmConfig { url: "http://npm".into(), email: Some("a@b".into()), token_env: None, letsencrypt_email: Some("a@b".into()) },
              docker: DockerConfig::default(),
              forward_host: ForwardHostConfig::default(),
              reconciler: ReconcilerConfig::default(),
              cleanup: CleanupConfig::default(),
              cloudflare: CloudflareConfig::default(),
              defaults: Defaults::default(),
              logging: LoggingConfig::default(),
          }
      }

      #[test]
      fn email_password_requires_npm_password() {
          let cfg = base_config();
          with_env(&[("NPM_PASSWORD", "")], || {
              std::env::remove_var("NPM_PASSWORD");
              let r = resolve_secrets(cfg.clone());
              assert!(matches!(r, Err(ConfigError::Secret(SecretError::Missing(_)))));
          });
      }

      #[test]
      fn email_password_happy_path() {
          let cfg = base_config();
          with_env(&[("NPM_PASSWORD", "hunter2"), ("CF_API_TOKEN", "tk")], || {
              let r = resolve_secrets(cfg.clone()).unwrap();
              matches!(r.npm_credential, NpmCredential::EmailPassword { .. });
              assert_eq!(r.cloudflare_global.as_deref(), Some("tk"));
          });
      }

      #[test]
      fn per_domain_cf_env_vars_resolved() {
          let mut cfg = base_config();
          cfg.cloudflare.domains.insert("example.com".into(), "CF_TOKEN_EX".into());
          with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "global"), ("CF_TOKEN_EX", "scoped")], || {
              let r = resolve_secrets(cfg.clone()).unwrap();
              assert_eq!(r.cloudflare_per_domain.get("example.com").unwrap(), "scoped");
          });
      }

      #[test]
      fn token_credential_happy_path() {
          let mut cfg = base_config();
          cfg.npm.email = None;
          cfg.npm.token_env = Some("NPM_TOKEN".into());
          with_env(&[("NPM_TOKEN", "jwt"), ("CF_API_TOKEN", "x")], || {
              let r = resolve_secrets(cfg.clone()).unwrap();
              matches!(r.npm_credential, NpmCredential::Token(_));
          });
      }

      #[test]
      fn ssl_default_with_no_cf_token_fails_validation() {
          let cfg = base_config();
          // no CF_API_TOKEN set, no per-domain overrides, defaults.ssl = true
          with_env(&[("NPM_PASSWORD", "x")], || {
              std::env::remove_var("CF_API_TOKEN");
              assert!(matches!(resolve_secrets(cfg.clone()), Err(ConfigError::Validation(_))));
          });
      }
  }
  ```

- [ ] **Step 3: Run — expect failures.** `cargo test --lib config`.

- [ ] **Step 4: Implement `resolve_secrets`.**
  ```rust
  pub fn resolve_secrets(config: Config) -> Result<ResolvedConfig, ConfigError> {
      let npm_credential = if let Some(env_name) = &config.npm.token_env {
          NpmCredential::Token(secrets::require(env_name)?)
      } else if let Some(email) = &config.npm.email {
          let password = secrets::require("NPM_PASSWORD")?;
          NpmCredential::EmailPassword { email: email.clone(), password }
      } else {
          return Err(ConfigError::Validation("npm.email or npm.token_env required".into()));
      };

      let cloudflare_global = secrets::optional("CF_API_TOKEN");
      let mut cloudflare_per_domain = BTreeMap::new();
      for (domain, env_name) in &config.cloudflare.domains {
          cloudflare_per_domain.insert(domain.clone(), secrets::require(env_name)?);
      }
      if config.defaults.ssl
          && cloudflare_global.is_none()
          && cloudflare_per_domain.is_empty()
      {
          return Err(ConfigError::Validation(
              "defaults.ssl=true requires CF_API_TOKEN or per-domain cloudflare.domains".into(),
          ));
      }
      Ok(ResolvedConfig { config, npm_credential, cloudflare_global, cloudflare_per_domain })
  }
  ```

- [ ] **Step 5: Run.** `cargo test --lib config` — all green.

- [ ] **Step 6: Commit.**
  ```bash
  git add src/config/mod.rs src/config/secrets.rs
  git commit -m "feat: load TOML config with env-var secrets and validation"
  ```

---

## Task 10: Telemetry setup

**Files:**
- Modify: `src/telemetry.rs`

- [ ] **Step 1: Implement.**
  ```rust
  use crate::config::{LogFormat, LoggingConfig};
  use tracing_subscriber::{EnvFilter, fmt, prelude::*};

  pub fn init(cfg: &LoggingConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
      let filter = EnvFilter::try_from_default_env()
          .or_else(|_| EnvFilter::try_new(&cfg.level))
          .unwrap_or_else(|_| EnvFilter::new("info"));
      let registry = tracing_subscriber::registry().with(filter);
      match cfg.format {
          LogFormat::Json => registry.with(fmt::layer().json()).try_init()?,
          LogFormat::Pretty => registry.with(fmt::layer().pretty()).try_init()?,
      }
      Ok(())
  }
  ```

- [ ] **Step 2: Smoke test.** Add at the bottom:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn init_does_not_panic() {
          let _ = init(&LoggingConfig::default());
      }
  }
  ```

- [ ] **Step 3: Run.** `cargo test --lib telemetry`.

- [ ] **Step 4: Commit.**
  ```bash
  git add src/telemetry.rs
  git commit -m "feat: initialize structured logging via tracing-subscriber"
  ```

---

## Task 11: NPM auth client (wiremock TDD)

**Files:**
- Modify: `src/npm/auth.rs`, `tests/npm_auth.rs`

- [ ] **Step 1: Write integration test.** `tests/npm_auth.rs`:
  ```rust
  use npm_docker_sync::npm::auth::{AuthClient, Credential};
  use wiremock::matchers::{method, path, body_json};
  use wiremock::{Mock, MockServer, ResponseTemplate};
  use serde_json::json;

  #[tokio::test]
  async fn login_with_email_password_returns_token() {
      let server = MockServer::start().await;
      Mock::given(method("POST"))
          .and(path("/api/tokens"))
          .and(body_json(json!({ "identity": "a@b", "secret": "pw" })))
          .respond_with(ResponseTemplate::new(200).set_body_json(json!({
              "token": "jwt-abc",
              "expires": "2099-01-01T00:00:00Z"
          })))
          .mount(&server)
          .await;

      let client = AuthClient::new(reqwest::Client::new(), server.uri());
      let tok = client.login(&Credential::EmailPassword { email: "a@b".into(), password: "pw".into() })
          .await.unwrap();
      assert_eq!(tok, "jwt-abc");
  }

  #[tokio::test]
  async fn static_token_credential_returns_as_is() {
      let server = MockServer::start().await;
      let client = AuthClient::new(reqwest::Client::new(), server.uri());
      let tok = client.login(&Credential::Token("given".into())).await.unwrap();
      assert_eq!(tok, "given");
  }
  ```

- [ ] **Step 2: Run — expect compile failure.** `cargo test --test npm_auth`.

- [ ] **Step 3: Implement `src/npm/auth.rs`.**
  ```rust
  use reqwest::Client;
  use serde::Deserialize;
  use thiserror::Error;

  #[derive(Debug, Clone)]
  pub enum Credential {
      EmailPassword { email: String, password: String },
      Token(String),
  }

  #[derive(Debug, Error)]
  pub enum AuthError {
      #[error("http: {0}")] Http(#[from] reqwest::Error),
      #[error("unexpected status {0}")] Status(u16),
  }

  pub struct AuthClient { client: Client, base: String }

  impl AuthClient {
      pub fn new(client: Client, base: String) -> Self { Self { client, base } }

      pub async fn login(&self, cred: &Credential) -> Result<String, AuthError> {
          match cred {
              Credential::Token(t) => Ok(t.clone()),
              Credential::EmailPassword { email, password } => {
                  #[derive(Deserialize)] struct Resp { token: String }
                  let res = self.client.post(format!("{}/api/tokens", self.base))
                      .json(&serde_json::json!({ "identity": email, "secret": password }))
                      .send().await?;
                  if !res.status().is_success() { return Err(AuthError::Status(res.status().as_u16())); }
                  Ok(res.json::<Resp>().await?.token)
              }
          }
      }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --test npm_auth` — both green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/npm/auth.rs tests/npm_auth.rs
  git commit -m "feat: authenticate with NPM via email/password or static token"
  ```

---

## Task 12: NPM proxy-host CRUD (wiremock TDD)

**Files:**
- Modify: `src/npm/proxy_hosts.rs`, `tests/npm_proxy_hosts.rs`

- [ ] **Step 1: Write integration tests** for list, create, update, delete against wiremock. Each test asserts NPM receives the correct request (path, method, body keys) and the parsed response round-trips through our types.

  Sketch the test file:
  ```rust
  use npm_docker_sync::npm::proxy_hosts::ProxyHostClient;
  use npm_docker_sync::npm::types::*;
  use serde_json::json;
  use wiremock::{Mock, MockServer, ResponseTemplate};
  use wiremock::matchers::{method, path, header};

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
              "id": 1, "domain_names": ["x.example.com"],
              "forward_scheme": "http", "forward_host": "svc", "forward_port": 80,
              "certificate_id": null, "ssl_forced": false, "caching_enabled": false,
              "allow_websocket_upgrade": true, "block_exploits": true, "meta": {}
          }])))
          .mount(&server).await;

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
              "id": 42, "domain_names": ["x.example.com"],
              "forward_scheme": "http", "forward_host": "svc", "forward_port": 80,
              "certificate_id": null, "ssl_forced": false, "caching_enabled": false,
              "allow_websocket_upgrade": true, "block_exploits": true, "meta": {}
          })))
          .mount(&server).await;

      let req = CreateProxyHost {
          domain_names: vec!["x.example.com".into()],
          forward_scheme: "http".into(), forward_host: "svc".into(), forward_port: 80,
          allow_websocket_upgrade: true, block_exploits: true, caching_enabled: false,
          meta: json!({ "npm_docker_sync": { "container_id": "abc" } }),
          certificate_id: 0, ssl_forced: false, access_list_id: 0,
      };
      assert_eq!(c.create(&req).await.unwrap().id, 42);
  }

  #[tokio::test]
  async fn update_sends_patch() {
      let (server, c) = start().await;
      Mock::given(method("PUT"))
          .and(path("/api/nginx/proxy-hosts/42"))
          .respond_with(ResponseTemplate::new(200).set_body_json(json!({
              "id": 42, "domain_names": ["x.example.com"],
              "forward_scheme": "https", "forward_host": "svc", "forward_port": 443,
              "certificate_id": null, "ssl_forced": false, "caching_enabled": false,
              "allow_websocket_upgrade": true, "block_exploits": true, "meta": {}
          })))
          .mount(&server).await;

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
          .mount(&server).await;
      c.delete(42).await.unwrap();
  }
  ```

- [ ] **Step 2: Run — expect compile failure.** `cargo test --test npm_proxy_hosts`.

- [ ] **Step 3: Implement `src/npm/proxy_hosts.rs`.**
  ```rust
  use crate::npm::types::*;
  use reqwest::Client;
  use thiserror::Error;

  #[derive(Debug, Error)]
  pub enum ProxyHostError {
      #[error("http: {0}")] Http(#[from] reqwest::Error),
      #[error("status {0}")] Status(u16),
  }

  pub struct ProxyHostClient { client: Client, base: String, token: String }

  impl ProxyHostClient {
      pub fn new(client: Client, base: String, token: String) -> Self { Self { client, base, token } }

      pub async fn list(&self) -> Result<Vec<ProxyHost>, ProxyHostError> {
          let res = self.client.get(format!("{}/api/nginx/proxy-hosts", self.base))
              .bearer_auth(&self.token).send().await?;
          if !res.status().is_success() { return Err(ProxyHostError::Status(res.status().as_u16())); }
          Ok(res.json().await?)
      }

      pub async fn create(&self, body: &CreateProxyHost) -> Result<ProxyHost, ProxyHostError> {
          let res = self.client.post(format!("{}/api/nginx/proxy-hosts", self.base))
              .bearer_auth(&self.token).json(body).send().await?;
          if !res.status().is_success() { return Err(ProxyHostError::Status(res.status().as_u16())); }
          Ok(res.json().await?)
      }

      pub async fn update(&self, id: u64, body: &UpdateProxyHost) -> Result<ProxyHost, ProxyHostError> {
          let res = self.client.put(format!("{}/api/nginx/proxy-hosts/{id}", self.base))
              .bearer_auth(&self.token).json(&body.0).send().await?;
          if !res.status().is_success() { return Err(ProxyHostError::Status(res.status().as_u16())); }
          Ok(res.json().await?)
      }

      pub async fn delete(&self, id: u64) -> Result<(), ProxyHostError> {
          let res = self.client.delete(format!("{}/api/nginx/proxy-hosts/{id}", self.base))
              .bearer_auth(&self.token).send().await?;
          if !res.status().is_success() { return Err(ProxyHostError::Status(res.status().as_u16())); }
          Ok(())
      }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --test npm_proxy_hosts` — all green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/npm/proxy_hosts.rs tests/npm_proxy_hosts.rs
  git commit -m "feat: NPM proxy-host CRUD client"
  ```

---

## Task 13: NPM certificates (LE request via wiremock)

**Files:**
- Modify: `src/npm/certificates.rs`, `tests/npm_certificates.rs`

- [ ] **Step 1: Test.** `tests/npm_certificates.rs`:
  ```rust
  use npm_docker_sync::npm::certificates::{CertificatesClient, LetsEncryptRequest};
  use wiremock::{Mock, MockServer, ResponseTemplate};
  use wiremock::matchers::{method, path};
  use serde_json::json;

  #[tokio::test]
  async fn request_creates_cert_and_returns_id() {
      let server = MockServer::start().await;
      Mock::given(method("POST"))
          .and(path("/api/nginx/certificates"))
          .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 7 })))
          .mount(&server).await;
      Mock::given(method("POST"))
          .and(path("/api/nginx/proxy-hosts/42/enable"))
          .respond_with(ResponseTemplate::new(200))
          .mount(&server).await;

      let c = CertificatesClient::new(reqwest::Client::new(), server.uri(), "jwt".into());
      let id = c.request_letsencrypt(&LetsEncryptRequest {
          domain: "x.example.com".into(),
          letsencrypt_email: "a@b".into(),
          dns_provider_credentials: "dns_cloudflare_api_token = tk\n".into(),
      }).await.unwrap();
      assert_eq!(id, 7);
  }
  ```

- [ ] **Step 2: Run — expect compile failure.** `cargo test --test npm_certificates`.

- [ ] **Step 3: Implement.** `src/npm/certificates.rs`:
  ```rust
  use reqwest::Client;
  use serde::Deserialize;
  use thiserror::Error;

  pub struct LetsEncryptRequest {
      pub domain: String,
      pub letsencrypt_email: String,
      /// Full `dns_cloudflare_api_token = ...` INI snippet passed to certbot.
      pub dns_provider_credentials: String,
  }

  #[derive(Debug, Error)]
  pub enum CertError {
      #[error("http: {0}")] Http(#[from] reqwest::Error),
      #[error("status {0}")] Status(u16),
  }

  pub struct CertificatesClient { client: Client, base: String, token: String }

  impl CertificatesClient {
      pub fn new(client: Client, base: String, token: String) -> Self { Self { client, base, token } }

      pub async fn request_letsencrypt(&self, req: &LetsEncryptRequest) -> Result<u64, CertError> {
          #[derive(Deserialize)] struct Resp { id: u64 }
          let body = serde_json::json!({
              "provider": "letsencrypt",
              "domain_names": [req.domain],
              "meta": {
                  "letsencrypt_email": req.letsencrypt_email,
                  "letsencrypt_agree": true,
                  "dns_challenge": true,
                  "dns_provider": "cloudflare",
                  "dns_provider_credentials": req.dns_provider_credentials,
                  "propagation_seconds": 30
              }
          });
          let res = self.client.post(format!("{}/api/nginx/certificates", self.base))
              .bearer_auth(&self.token).json(&body).send().await?;
          if !res.status().is_success() { return Err(CertError::Status(res.status().as_u16())); }
          Ok(res.json::<Resp>().await?.id)
      }
  }
  ```

- [ ] **Step 4: Run.** `cargo test --test npm_certificates` — green.

- [ ] **Step 5: Commit.**
  ```bash
  git add src/npm/certificates.rs tests/npm_certificates.rs
  git commit -m "feat: request Let's Encrypt cert via NPM"
  ```

---

## Task 14: Compose `NpmClient`

**Files:**
- Modify: `src/npm/mod.rs`

- [ ] **Step 1: Implement.** `NpmClient` owns the reqwest client and the three sub-clients, transparently handles 401 re-login:
  ```rust
  pub mod auth;
  pub mod certificates;
  pub mod meta;
  pub mod proxy_hosts;
  pub mod types;

  use crate::npm::auth::{AuthClient, AuthError, Credential};
  use crate::npm::certificates::{CertificatesClient, CertError, LetsEncryptRequest};
  use crate::npm::proxy_hosts::{ProxyHostClient, ProxyHostError};
  use crate::npm::types::*;
  use reqwest::Client;
  use std::sync::RwLock;
  use thiserror::Error;

  #[derive(Debug, Error)]
  pub enum NpmError {
      #[error(transparent)] Auth(#[from] AuthError),
      #[error(transparent)] ProxyHost(#[from] ProxyHostError),
      #[error(transparent)] Cert(#[from] CertError),
  }

  pub struct NpmClient {
      http: Client,
      base: String,
      cred: Credential,
      auth: AuthClient,
      token: RwLock<String>,
  }

  impl NpmClient {
      pub async fn connect(base: String, cred: Credential) -> Result<Self, NpmError> {
          let http = Client::builder().build().expect("reqwest client");
          let auth = AuthClient::new(http.clone(), base.clone());
          let token = auth.login(&cred).await?;
          Ok(Self { http, base, cred, auth, token: RwLock::new(token) })
      }

      pub async fn list_proxy_hosts(&self) -> Result<Vec<ProxyHost>, NpmError> {
          let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token.read().unwrap().clone());
          Ok(c.list().await?)
      }

      pub async fn create_proxy_host(&self, body: &CreateProxyHost) -> Result<ProxyHost, NpmError> {
          let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token.read().unwrap().clone());
          Ok(c.create(body).await?)
      }

      pub async fn update_proxy_host(&self, id: u64, body: &UpdateProxyHost) -> Result<ProxyHost, NpmError> {
          let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token.read().unwrap().clone());
          Ok(c.update(id, body).await?)
      }

      pub async fn delete_proxy_host(&self, id: u64) -> Result<(), NpmError> {
          let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token.read().unwrap().clone());
          Ok(c.delete(id).await?)
      }

      pub async fn request_certificate(&self, req: &LetsEncryptRequest) -> Result<u64, NpmError> {
          let c = CertificatesClient::new(self.http.clone(), self.base.clone(), self.token.read().unwrap().clone());
          Ok(c.request_letsencrypt(req).await?)
      }

      /// Called by retry wrapper when a 401 surfaces — refreshes the stored token.
      pub async fn refresh_token(&self) -> Result<(), NpmError> {
          let t = self.auth.login(&self.cred).await?;
          *self.token.write().unwrap() = t;
          Ok(())
      }
  }
  ```

- [ ] **Step 2: Build and run existing tests.** `cargo test` — all previous tests still green.

- [ ] **Step 3: Commit.**
  ```bash
  git add src/npm/mod.rs
  git commit -m "feat: compose NpmClient from auth, hosts, certificates"
  ```

---

## Task 15: Retry wrapper (tokio TDD)

**Files:**
- Modify: `src/writer/retry.rs`

- [ ] **Step 1: Design.** `retry_call` takes a closure returning a future, an error classifier, a 401 hook for re-login, and runs up to 3 attempts with exponential backoff (1s, 4s + jitter). 401 triggers one refresh and retries without consuming an attempt.

- [ ] **Step 2: Write failing tests.**
  ```rust
  use std::sync::Arc;
  use std::sync::atomic::{AtomicU32, Ordering};
  use std::time::Duration;

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum FailKind { Transient, NonTransient, Unauthorized }

  pub struct RetryPolicy { pub max_attempts: u32, pub base: Duration }
  impl Default for RetryPolicy {
      fn default() -> Self { Self { max_attempts: 3, base: Duration::from_secs(1) } }
  }

  #[derive(Debug, thiserror::Error)]
  pub enum RetryError<E> {
      #[error("exhausted retries: {0}")] Exhausted(E),
      #[error("non-transient: {0}")] NonTransient(E),
  }

  pub async fn retry_call<F, Fut, T, E, Classify, OnUnauth, FutU>(
      policy: &RetryPolicy,
      classify: Classify,
      mut on_unauthorized: OnUnauth,
      mut f: F,
  ) -> Result<T, RetryError<E>>
  where
      F: FnMut() -> Fut,
      Fut: std::future::Future<Output = Result<T, E>>,
      Classify: Fn(&E) -> FailKind,
      OnUnauth: FnMut() -> FutU,
      FutU: std::future::Future<Output = ()>,
  {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[tokio::test(start_paused = true)]
      async fn succeeds_on_first_attempt() {
          let counter = Arc::new(AtomicU32::new(0));
          let c = counter.clone();
          let res: Result<i32, RetryError<&str>> = retry_call(
              &RetryPolicy::default(),
              |_| FailKind::Transient,
              || async {},
              || { let c = c.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Ok::<_, &str>(42) } },
          ).await;
          assert_eq!(res.unwrap(), 42);
          assert_eq!(counter.load(Ordering::SeqCst), 1);
      }

      #[tokio::test(start_paused = true)]
      async fn retries_on_transient_then_gives_up() {
          let counter = Arc::new(AtomicU32::new(0));
          let c = counter.clone();
          let res: Result<i32, RetryError<&str>> = retry_call(
              &RetryPolicy::default(),
              |_| FailKind::Transient,
              || async {},
              || { let c = c.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Err("boom") } },
          ).await;
          assert!(matches!(res, Err(RetryError::Exhausted(_))));
          assert_eq!(counter.load(Ordering::SeqCst), 3);
      }

      #[tokio::test(start_paused = true)]
      async fn does_not_retry_non_transient() {
          let counter = Arc::new(AtomicU32::new(0));
          let c = counter.clone();
          let res: Result<i32, RetryError<&str>> = retry_call(
              &RetryPolicy::default(),
              |_| FailKind::NonTransient,
              || async {},
              || { let c = c.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Err("no") } },
          ).await;
          assert!(matches!(res, Err(RetryError::NonTransient(_))));
          assert_eq!(counter.load(Ordering::SeqCst), 1);
      }

      #[tokio::test(start_paused = true)]
      async fn unauthorized_refreshes_without_burning_attempts() {
          let call_count = Arc::new(AtomicU32::new(0));
          let refresh_count = Arc::new(AtomicU32::new(0));
          let cc = call_count.clone();
          let rc = refresh_count.clone();
          let res: Result<i32, RetryError<&str>> = retry_call(
              &RetryPolicy::default(),
              |e| if *e == "401" { FailKind::Unauthorized } else { FailKind::NonTransient },
              || { let rc = rc.clone(); async move { rc.fetch_add(1, Ordering::SeqCst); } },
              || {
                  let cc = cc.clone();
                  async move {
                      let n = cc.fetch_add(1, Ordering::SeqCst);
                      if n == 0 { Err("401") } else { Ok(7) }
                  }
              },
          ).await;
          assert_eq!(res.unwrap(), 7);
          assert_eq!(refresh_count.load(Ordering::SeqCst), 1);
      }
  }
  ```

- [ ] **Step 3: Run — expect failures.** `cargo test --lib writer::retry`.

- [ ] **Step 4: Implement.**
  ```rust
  use rand::Rng;

  pub async fn retry_call<F, Fut, T, E, Classify, OnUnauth, FutU>(
      policy: &RetryPolicy,
      classify: Classify,
      mut on_unauthorized: OnUnauth,
      mut f: F,
  ) -> Result<T, RetryError<E>>
  where
      F: FnMut() -> Fut,
      Fut: std::future::Future<Output = Result<T, E>>,
      Classify: Fn(&E) -> FailKind,
      OnUnauth: FnMut() -> FutU,
      FutU: std::future::Future<Output = ()>,
  {
      let mut attempt: u32 = 0;
      loop {
          let result = f().await;
          match result {
              Ok(v) => return Ok(v),
              Err(e) => match classify(&e) {
                  FailKind::NonTransient => return Err(RetryError::NonTransient(e)),
                  FailKind::Unauthorized => {
                      on_unauthorized().await;
                      // do not consume attempt budget
                      continue;
                  }
                  FailKind::Transient => {
                      attempt += 1;
                      if attempt >= policy.max_attempts { return Err(RetryError::Exhausted(e)); }
                      let base_secs = policy.base.as_secs_f64();
                      let backoff = base_secs * 4.0_f64.powi((attempt - 1) as i32);
                      let jitter = rand::thread_rng().gen_range(0.0..0.3);
                      tokio::time::sleep(Duration::from_secs_f64(backoff + jitter)).await;
                  }
              },
          }
      }
  }
  ```

- [ ] **Step 5: Run.** `cargo test --lib writer::retry` — all green.

- [ ] **Step 6: Commit.**
  ```bash
  git add src/writer/retry.rs
  git commit -m "feat: retry wrapper with exponential backoff and 401 hook"
  ```

---

## Task 16: Docker client wrapper

**Files:**
- Modify: `src/docker/mod.rs`

- [ ] **Step 1: Implement `DockerClient`.** Thin wrapper around `bollard` exposing only what the watcher + reconciler need. Keep it async. Feature-gate live Docker tests so `cargo test` doesn't require Docker:
  ```rust
  pub mod labels;
  pub mod spec;
  pub mod watcher;

  use bollard::Docker;
  use bollard::container::{ListContainersOptions, InspectContainerOptions};
  use bollard::secret::ContainerSummary;
  use std::collections::{BTreeMap, HashMap};
  use thiserror::Error;

  #[derive(Debug, Error)]
  pub enum DockerError {
      #[error("docker: {0}")] Docker(#[from] bollard::errors::Error),
  }

  pub struct DockerClient { inner: Docker }

  impl DockerClient {
      pub fn connect(socket: &str) -> Result<Self, DockerError> {
          let inner = if socket.starts_with("tcp://") {
              Docker::connect_with_http(socket, 5, bollard::API_DEFAULT_VERSION)?
          } else {
              Docker::connect_with_unix(socket, 5, bollard::API_DEFAULT_VERSION)?
          };
          Ok(Self { inner })
      }

      pub fn handle(&self) -> &Docker { &self.inner }

      pub async fn list_labeled(&self) -> Result<Vec<ContainerSummary>, DockerError> {
          let mut filters: HashMap<String, Vec<String>> = HashMap::new();
          filters.insert("label".into(), vec!["nginx_proxy_url".into()]);
          let opts = ListContainersOptions { all: true, filters, ..Default::default() };
          Ok(self.inner.list_containers(Some(opts)).await?)
      }

      pub async fn inspect(&self, id: &str) -> Result<bollard::secret::ContainerInspectResponse, DockerError> {
          Ok(self.inner.inspect_container(id, None::<InspectContainerOptions>).await?)
      }
  }
  ```

- [ ] **Step 2: Build.** `cargo build` — passes.

- [ ] **Step 3: Commit.**
  ```bash
  git add src/docker/mod.rs
  git commit -m "feat: Docker client wrapper around bollard"
  ```

---

## Task 17: Docker watcher task

**Files:**
- Modify: `src/docker/watcher.rs`

Watcher subscribes to Docker events, converts each event into an `Intent`, and sends it on the channel. Reconnects with backoff on stream failure.

- [ ] **Step 1: Implement.** Sketch:
  ```rust
  use crate::docker::{DockerClient, labels, spec::ContainerSpec};
  use crate::intent::Intent;
  use bollard::system::EventsOptions;
  use futures_util::StreamExt;
  use std::collections::HashMap;
  use std::time::Duration;
  use tokio::sync::mpsc;
  use tokio_util::sync::CancellationToken;

  pub struct Watcher {
      docker: DockerClient,
      defaults: labels::Defaults,
      tx: mpsc::Sender<Intent>,
      cancel: CancellationToken,
  }

  impl Watcher {
      pub fn new(docker: DockerClient, defaults: labels::Defaults, tx: mpsc::Sender<Intent>, cancel: CancellationToken) -> Self {
          Self { docker, defaults, tx, cancel }
      }

      pub async fn run(self) {
          let mut backoff = Duration::from_secs(1);
          loop {
              if self.cancel.is_cancelled() { return; }
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
          filters.insert("type".into(), vec!["container".into()]);
          filters.insert("event".into(), vec!["start".into(), "die".into(), "destroy".into()]);
          let mut stream = self.docker.handle().events(Some(EventsOptions::<String> { filters, ..Default::default() }));
          while let Some(ev) = stream.next().await {
              let ev = ev?;
              if self.cancel.is_cancelled() { return Ok(()); }
              let Some(actor) = ev.actor else { continue };
              let Some(id) = actor.id else { continue };
              let action = ev.action.as_deref().unwrap_or("");
              match action {
                  "start" => {
                      if let Some(spec) = self.build_spec(&id).await {
                          let _ = self.tx.send(Intent::Upsert { container_id: id, spec }).await;
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
          let insp = match self.docker.inspect(id).await { Ok(v) => v, Err(_) => return None };
          let config = insp.config?;
          let raw_labels = config.labels.unwrap_or_default();
          let labels_map: std::collections::BTreeMap<String, String> = raw_labels.into_iter().collect();
          let name = insp.name.as_deref().unwrap_or(id).trim_start_matches('/').to_string();
          let mut exposed_ports: Vec<u16> = config.exposed_ports.unwrap_or_default().keys()
              .filter_map(|p| p.split('/').next().and_then(|n| n.parse::<u16>().ok()))
              .collect();
          exposed_ports.sort_unstable(); exposed_ports.dedup();
          let mut aliases = std::collections::BTreeMap::new();
          if let Some(ns) = insp.network_settings.as_ref().and_then(|n| n.networks.clone()) {
              for (net, net_cfg) in ns {
                  if let Some(ip) = net_cfg.ip_address { aliases.insert(format!("{net}:ip"), ip); }
                  if let Some(ref ar) = net_cfg.aliases { for a in ar { aliases.insert(format!("{net}:alias"), a.clone()); } }
              }
          }
          let facts = labels::ContainerFacts {
              id, name: &name,
              labels: &labels_map,
              exposed_ports: &exposed_ports,
              network_aliases: aliases,
          };
          match labels::parse(facts, &self.defaults) {
              Ok(s) => Some(s),
              Err(labels::LabelError::NotLabeled) => None,
              Err(e) => { tracing::warn!(error = %e, container = %id, "skip: bad label"); None }
          }
      }
  }
  ```

- [ ] **Step 2: Build.** `cargo build`.

- [ ] **Step 3: Commit.**
  ```bash
  git add src/docker/watcher.rs
  git commit -m "feat: Docker event watcher task emitting intents"
  ```

---

## Task 18: NPM writer actor

**Files:**
- Modify: `src/writer/mod.rs`, `tests/writer_actor.rs`

- [ ] **Step 1: Write integration tests** covering: upsert creates + requests cert, upsert idempotent when no diff, upsert updates on diff, conflict when host exists with different container_id, remove respects cleanup flag, remove refuses hosts without marker.

  `tests/writer_actor.rs` (sketch of the first test — add similar coverage for the remaining cases):
  ```rust
  use npm_docker_sync::config::ForwardStrategy;
  use npm_docker_sync::docker::spec::{ContainerSpec, Scheme};
  use npm_docker_sync::intent::Intent;
  use npm_docker_sync::npm::NpmClient;
  use npm_docker_sync::npm::auth::Credential;
  use npm_docker_sync::writer::{NpmWriter, WriterConfig};
  use serde_json::json;
  use std::collections::BTreeMap;
  use std::time::Duration;
  use tokio::sync::mpsc;
  use tokio_util::sync::CancellationToken;
  use wiremock::{Mock, MockServer, ResponseTemplate};
  use wiremock::matchers::{method, path};

  #[tokio::test]
  async fn upsert_creates_host_and_requests_certificate() {
      let server = MockServer::start().await;
      Mock::given(method("POST")).and(path("/api/tokens"))
          .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "token": "jwt", "expires": "2099-01-01T00:00:00Z" })))
          .mount(&server).await;
      Mock::given(method("GET")).and(path("/api/nginx/proxy-hosts"))
          .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
          .mount(&server).await;
      Mock::given(method("POST")).and(path("/api/nginx/proxy-hosts"))
          .respond_with(ResponseTemplate::new(201).set_body_json(json!({
              "id": 1, "domain_names": ["app.example.com"],
              "forward_scheme": "http", "forward_host": "myapp", "forward_port": 3000,
              "certificate_id": null, "ssl_forced": false, "caching_enabled": false,
              "allow_websocket_upgrade": true, "block_exploits": true,
              "meta": { "npm_docker_sync": { "version": 1, "container_id": "abc", "container_name": "myapp", "managed_at": "2026-04-21T00:00:00Z" } }
          })))
          .mount(&server).await;
      Mock::given(method("POST")).and(path("/api/nginx/certificates"))
          .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9 })))
          .expect(1)
          .mount(&server).await;

      let npm = NpmClient::connect(server.uri(), Credential::EmailPassword { email: "a".into(), password: "b".into() }).await.unwrap();
      let (tx, rx) = mpsc::channel(16);
      let cancel = CancellationToken::new();
      let cfg = WriterConfig {
          forward_strategy: ForwardStrategy::ContainerName,
          host_address: None,
          network: None,
          letsencrypt_email: "a@b".into(),
          cleanup_on_remove: true,
          cf_global: Some("tk".into()),
          cf_per_domain: BTreeMap::new(),
      };
      let handle = tokio::spawn(NpmWriter::new(npm, cfg, rx, cancel.clone()).run());
      tx.send(Intent::Upsert {
          container_id: "abc".into(),
          spec: ContainerSpec {
              id: "abc".into(), name: "myapp".into(), url: "app.example.com".into(),
              port: 3000, scheme: Scheme::Http, ssl: true,
              websockets: true, block_exploits: true,
              network_aliases: BTreeMap::new(),
          },
      }).await.unwrap();
      tokio::time::sleep(Duration::from_millis(100)).await;
      cancel.cancel();
      handle.await.unwrap();
      server.verify().await;
  }
  ```

- [ ] **Step 2: Implement writer.** `src/writer/mod.rs`:
  ```rust
  pub mod diff;
  pub mod retry;

  use crate::cloudflare::TokenResolver;
  use crate::config::ForwardStrategy;
  use crate::docker::spec::{ContainerSpec, Scheme};
  use crate::intent::Intent;
  use crate::npm::NpmClient;
  use crate::npm::certificates::LetsEncryptRequest;
  use crate::npm::meta::{self, OwnershipMarker};
  use crate::npm::types::{CreateProxyHost, ProxyHost};
  use crate::writer::diff::{DesiredProxyHost, diff_spec};
  use std::collections::{BTreeMap, HashMap};
  use time::OffsetDateTime;
  use tokio::sync::mpsc;
  use tokio_util::sync::CancellationToken;

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
      pub fn new(npm: NpmClient, cfg: WriterConfig, rx: mpsc::Receiver<Intent>, cancel: CancellationToken) -> Self {
          let cf = TokenResolver::new(cfg.cf_global.clone(), cfg.cf_per_domain.clone());
          Self { npm, cfg, rx, cancel, cache: HashMap::new(), cf }
      }

      pub async fn run(mut self) {
          // Seed cache from NPM on start.
          if let Ok(hosts) = self.npm.list_proxy_hosts().await {
              for h in hosts {
                  if let Some(m) = meta::decode(&h.meta) {
                      self.cache.insert(m.container_id, h.id);
                  }
              }
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

      async fn handle(&mut self, intent: Intent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
                  key.and_then(|k| spec.network_aliases.get(&k).cloned()).unwrap_or_else(|| spec.name.clone())
              }
              ForwardStrategy::HostPort => self.cfg.host_address.clone().unwrap_or_else(|| "host.docker.internal".into()),
          }
      }

      async fn upsert(&mut self, container_id: &str, spec: &ContainerSpec) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
          let forward_host = self.forward_host_for(spec);
          let hosts = self.npm.list_proxy_hosts().await?;
          let existing = hosts.iter().find(|h| h.domain_names.iter().any(|d| d == &spec.url));

          match existing {
              Some(h) => {
                  let marker = meta::decode(&h.meta);
                  if marker.as_ref().map(|m| m.container_id.as_str()) != Some(container_id) {
                      tracing::error!(url = %spec.url, host_id = h.id, "conflict: host exists for another owner, skipping");
                      return Ok(());
                  }
                  if let Some(patch) = diff_spec(h, &DesiredProxyHost { spec, forward_host: &forward_host }) {
                      self.npm.update_proxy_host(h.id, &patch).await?;
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
                      forward_scheme: match spec.scheme { Scheme::Http => "http", Scheme::Https => "https" }.into(),
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
                  let created = self.npm.create_proxy_host(&body).await?;
                  self.cache.insert(container_id.to_string(), created.id);

                  if spec.ssl {
                      let token = match self.cf.for_domain(&spec.url) {
                          Ok(t) => t.to_string(),
                          Err(e) => { tracing::error!(error = %e, "no CF token, skipping cert"); return Ok(()); }
                      };
                      let credentials = format!("dns_cloudflare_api_token = {token}\n");
                      let cert_id = self.npm.request_certificate(&LetsEncryptRequest {
                          domain: spec.url.clone(),
                          letsencrypt_email: self.cfg.letsencrypt_email.clone(),
                          dns_provider_credentials: credentials,
                      }).await?;
                      let mut patch = serde_json::Map::new();
                      patch.insert("certificate_id".into(), serde_json::Value::from(cert_id));
                      patch.insert("ssl_forced".into(), serde_json::Value::from(true));
                      self.npm.update_proxy_host(created.id, &crate::npm::types::UpdateProxyHost(patch)).await?;
                  }
              }
          }
          Ok(())
      }

      async fn remove(&mut self, container_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
          if !self.cfg.cleanup_on_remove { return Ok(()); }
          let host_id = match self.cache.remove(container_id) { Some(id) => id, None => return Ok(()) };
          let hosts = self.npm.list_proxy_hosts().await?;
          let Some(host) = hosts.iter().find(|h| h.id == host_id) else { return Ok(()); };
          let Some(marker) = meta::decode(&host.meta) else {
              tracing::warn!(host_id, "refuse to delete host without ownership marker");
              return Ok(());
          };
          if marker.container_id != container_id {
              tracing::warn!(host_id, "marker container_id mismatch; refusing to delete");
              return Ok(());
          }
          self.npm.delete_proxy_host(host_id).await?;
          Ok(())
      }
  }
  ```

- [ ] **Step 3: Run tests.** `cargo test --test writer_actor` — green.

- [ ] **Step 4: Commit.**
  ```bash
  git add src/writer/mod.rs tests/writer_actor.rs
  git commit -m "feat: NPM writer actor handling upsert and remove intents"
  ```

---

## Task 19: Reconciler task

**Files:**
- Modify: `src/reconciler/mod.rs`

- [ ] **Step 1: Implement.**
  ```rust
  pub mod plan;

  use crate::docker::{DockerClient, labels};
  use crate::intent::Intent;
  use crate::npm::NpmClient;
  use std::collections::BTreeMap;
  use std::time::Duration;
  use tokio::sync::mpsc;
  use tokio_util::sync::CancellationToken;

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
              let insp = match self.docker.inspect(&id).await { Ok(v) => v, Err(_) => continue };
              let Some(config) = insp.config else { continue };
              let raw = config.labels.unwrap_or_default();
              let labels_map: BTreeMap<String, String> = raw.into_iter().collect();
              let name = insp.name.as_deref().unwrap_or(&id).trim_start_matches('/').to_string();
              let mut ports: Vec<u16> = config.exposed_ports.unwrap_or_default().keys()
                  .filter_map(|p| p.split('/').next().and_then(|n| n.parse::<u16>().ok())).collect();
              ports.sort_unstable(); ports.dedup();
              let facts = labels::ContainerFacts {
                  id: &id, name: &name, labels: &labels_map,
                  exposed_ports: &ports,
                  network_aliases: BTreeMap::new(),
              };
              if let Ok(spec) = labels::parse(facts, &self.defaults) {
                  specs.push(spec);
              }
          }
          let hosts = self.npm.list_proxy_hosts().await?;
          for intent in plan::plan(&specs, &hosts) {
              let _ = self.tx.send(intent).await;
          }
          Ok(())
      }
  }
  ```

- [ ] **Step 2: Build.** `cargo build`.

- [ ] **Step 3: Commit.**
  ```bash
  git add src/reconciler/mod.rs
  git commit -m "feat: periodic reconciliation sweep task"
  ```

---

## Task 20: `main.rs` wiring + graceful shutdown

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement.**
  ```rust
  use anyhow::{Context, Result};
  use clap::Parser;
  use npm_docker_sync::{
      config::{self, ForwardStrategy, NpmCredential},
      docker::{DockerClient, labels::Defaults as LabelDefaults},
      docker::watcher::Watcher,
      intent::Intent,
      npm::NpmClient,
      npm::auth::Credential,
      reconciler::Reconciler,
      telemetry,
      writer::{NpmWriter, WriterConfig},
  };
  use std::{path::PathBuf, time::Duration};
  use tokio::{signal, sync::mpsc};
  use tokio_util::sync::CancellationToken;

  #[derive(Parser)]
  #[command(name = "npm-docker-sync")]
  struct Cli {
      #[arg(long, default_value = "/etc/npm-docker-sync/config.toml")]
      config: PathBuf,
  }

  #[tokio::main]
  async fn main() -> Result<()> {
      let cli = Cli::parse();
      let resolved = config::load(&cli.config).context("load config")?;
      telemetry::init(&resolved.config.logging).map_err(anyhow::Error::from_boxed)?;
      tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

      let cred = match &resolved.npm_credential {
          NpmCredential::EmailPassword { email, password } =>
              Credential::EmailPassword { email: email.clone(), password: password.clone() },
          NpmCredential::Token(t) => Credential::Token(t.clone()),
      };
      let npm = NpmClient::connect(resolved.config.npm.url.clone(), cred).await
          .context("connect to NPM")?;
      let docker = DockerClient::connect(&resolved.config.docker.socket).context("connect to Docker")?;

      let defaults = LabelDefaults {
          scheme: resolved.config.defaults.scheme,
          ssl: resolved.config.defaults.ssl,
          websockets: resolved.config.defaults.websockets,
          block_exploits: resolved.config.defaults.block_exploits,
      };
      let cancel = CancellationToken::new();
      let (tx, rx) = mpsc::channel::<Intent>(256);

      let writer_cfg = WriterConfig {
          forward_strategy: resolved.config.forward_host.strategy,
          host_address: resolved.config.forward_host.host_address.clone(),
          network: resolved.config.forward_host.network.clone(),
          letsencrypt_email: resolved.config.npm.letsencrypt_email.clone().unwrap_or_default(),
          cleanup_on_remove: resolved.config.cleanup.on_remove,
          cf_global: resolved.cloudflare_global.clone(),
          cf_per_domain: resolved.cloudflare_per_domain.clone(),
      };
      let writer = tokio::spawn(NpmWriter::new(npm.clone_for_tasks(), writer_cfg, rx, cancel.clone()).run());

      let recon = Reconciler {
          docker: docker.clone_for_tasks(),
          npm: npm.clone_for_tasks(),
          defaults: defaults.clone(),
          interval: Duration::from_secs(resolved.config.reconciler.interval_seconds),
          tx: tx.clone(),
          cancel: cancel.clone(),
      };
      let recon = tokio::spawn(recon.run());

      let watcher = Watcher::new(docker, defaults, tx, cancel.clone());
      let watcher = tokio::spawn(watcher.run());

      shutdown_signal().await;
      tracing::info!("shutdown requested");
      cancel.cancel();
      let _ = tokio::join!(writer, recon, watcher);
      Ok(())
  }

  async fn shutdown_signal() {
      let ctrl_c = async { let _ = signal::ctrl_c().await; };
      #[cfg(unix)]
      let term = async {
          let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate()).expect("SIGTERM");
          sig.recv().await;
      };
      #[cfg(not(unix))]
      let term = std::future::pending::<()>();
      tokio::select! { _ = ctrl_c => {}, _ = term => {} }
  }
  ```

- [ ] **Step 2: Add `clap` dependency.** Update `Cargo.toml`:
  ```toml
  clap = { version = "4", features = ["derive"] }
  ```

- [ ] **Step 3: Add `clone_for_tasks` helpers** on `DockerClient` and `NpmClient`. `DockerClient::clone_for_tasks` reuses `bollard::Docker` (cheap to clone). For `NpmClient`, share the same `RwLock` via `Arc`. Refactor `NpmClient` accordingly:
  ```rust
  // src/npm/mod.rs — change token: RwLock<String> to Arc<RwLock<String>>
  use std::sync::Arc;
  pub struct NpmClient { /* ...same... */ token: Arc<RwLock<String>>, /* ... */ }
  impl NpmClient { pub fn clone_for_tasks(&self) -> Self { /* clone all fields including Arc */ } }
  ```
  Similarly for `DockerClient`:
  ```rust
  impl DockerClient { pub fn clone_for_tasks(&self) -> Self { Self { inner: self.inner.clone() } } }
  ```

- [ ] **Step 4: Rebuild.** `cargo build` → success.

- [ ] **Step 5: Smoke test.** `cargo run -- --config nonexistent.toml` — expect fast, clear error about the file.

- [ ] **Step 6: Commit.**
  ```bash
  git add src/main.rs src/npm/mod.rs src/docker/mod.rs Cargo.toml Cargo.lock
  git commit -m "feat: wire main entry with graceful shutdown"
  ```

---

## Task 21: Example config, Dockerfile, compose example

**Files:**
- Create: `examples/config.toml`, `docker/Dockerfile`, `docker/docker-compose.example.yml`

- [ ] **Step 1: `examples/config.toml`** — the annotated spec from Section 4 of the design doc.

- [ ] **Step 2: `docker/Dockerfile`** — multi-stage, distroless:
  ```dockerfile
  # syntax=docker/dockerfile:1.6
  FROM rust:1-slim AS builder
  WORKDIR /src
  COPY Cargo.toml Cargo.lock ./
  COPY src ./src
  RUN --mount=type=cache,target=/usr/local/cargo/registry \
      --mount=type=cache,target=/src/target \
      cargo build --release && cp target/release/npm-docker-sync /tmp/bin

  FROM gcr.io/distroless/cc-debian12:nonroot
  COPY --from=builder /tmp/bin /npm-docker-sync
  USER nonroot
  ENTRYPOINT ["/npm-docker-sync"]
  CMD ["--config", "/etc/npm-docker-sync/config.toml"]
  ```

- [ ] **Step 3: `docker/docker-compose.example.yml`.**
  ```yaml
  services:
    npm:
      image: jc21/nginx-proxy-manager:latest
      ports: ["80:80", "81:81", "443:443"]
      networks: [proxy]
      volumes: ["npm_data:/data", "npm_letsencrypt:/etc/letsencrypt"]

    npm-docker-sync:
      build: { context: .., dockerfile: docker/Dockerfile }
      depends_on: [npm]
      environment:
        NPM_PASSWORD: "${NPM_PASSWORD:?set in .env}"
        CF_API_TOKEN: "${CF_API_TOKEN:?set in .env}"
      volumes:
        - "/var/run/docker.sock:/var/run/docker.sock:ro"
        - "./config.toml:/etc/npm-docker-sync/config.toml:ro"
      networks: [proxy]

  networks:
    proxy:
      external: false

  volumes:
    npm_data: {}
    npm_letsencrypt: {}
  ```

- [ ] **Step 4: Build image.** `docker build -f docker/Dockerfile -t npm-docker-sync:dev .` — success.

- [ ] **Step 5: Commit.**
  ```bash
  git add examples/config.toml docker/Dockerfile docker/docker-compose.example.yml
  git commit -m "chore: add example config and Docker packaging"
  ```

---

## Task 22: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write workflow** (fmt, clippy, test) for push + PR to `develop`/`main`:
  ```yaml
  name: CI
  on:
    push:
      branches: [main, develop, "feature/**"]
    pull_request:
      branches: [main, develop]

  jobs:
    fmt:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
          with: { components: rustfmt }
        - run: cargo fmt --check
    clippy:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
          with: { components: clippy }
        - uses: Swatinem/rust-cache@v2
        - run: cargo clippy --all-targets -- -D warnings
    test:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo test --all-targets
  ```

- [ ] **Step 2: Commit.**
  ```bash
  git add .github/workflows/ci.yml
  git commit -m "ci: add fmt, clippy, and test workflow"
  ```

---

## Task 23: README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write a concise README** covering: what it does, how to configure labels, how to configure the service (TOML + env vars), how to run via Docker compose, how to build locally.

- [ ] **Step 2: Commit.**
  ```bash
  git add README.md
  git commit -m "docs: add README with usage and configuration"
  ```

---

## Task 24: Final verification

- [ ] **Step 1: Full test run.**
  ```bash
  cargo test --all-targets
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```
  All must pass.

- [ ] **Step 2: Build release binary.**
  ```bash
  cargo build --release
  ```

- [ ] **Step 3: Report task summary** to the user.

---

## Execution notes

- **No** `Co-Authored-By` trailers in commit messages.
- Every `git add` names explicit files. No `git add .`.
- Every commit message uses a conventional prefix (`feat:`, `fix:`, `test:`, `chore:`, `docs:`, `refactor:`, `ci:`).
- All work stays on `feature/initial-implementation`. Merging to `develop` is a separate, user-driven step.
- For each task, run the verification step before committing. If a compile or test fails, do not commit — fix and re-run.

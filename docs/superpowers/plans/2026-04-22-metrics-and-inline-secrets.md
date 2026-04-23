# Metrics and Inline-Secrets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in Prometheus metrics endpoint (with Grafana dashboard) and inline-secret support in the TOML config.

**Architecture:** Metrics via `metrics` facade + `metrics-exporter-prometheus` (built-in HTTP listener, no new HTTP framework). Instrumentation lives in the retry wrapper, writer, and reconciler — all gated at compile-time by being no-ops when no recorder is installed. Secrets schema adopts "TOML names the source" — each secret field is either `field = "literal"` or `field_env = "NAME"`, never both, never with ambient env-var override.

**Tech Stack:** Rust 2024 edition, `metrics` 0.24+, `metrics-exporter-prometheus` 0.17+ (http-listener feature), extends existing tokio/reqwest/serde stack.

**Spec:** `docs/superpowers/specs/2026-04-22-metrics-and-inline-secrets-design.md`

---

## Before you begin

- **Branch.** Already on `feature/metrics-and-inline-secrets`, off `develop`. The spec commit (`b6eec76`) is present.
- **Dependency versions.** User's CLAUDE.md mandates latest stable. Before Task 1, resolve current versions of `metrics` and `metrics-exporter-prometheus` via context7 (`mcp__context7__resolve-library-id` + `mcp__context7__query-docs`). The exporter crate has had API changes across 0.15→0.16→0.17; the plan is written against 0.17 but adapt as needed.
- **Conventional commits.** Prefixes per CLAUDE.md: `feat:`, `fix:`, `test:`, `chore:`, `docs:`, `refactor:`. No `Co-Authored-By`. Specific-file `git add` only.
- **TDD.** Pure logic (secrets validation) gets full TDD. Instrumentation changes get integration tests via a `metrics::debugging::Snapshotter` or equivalent — see the metrics crate docs.

---

## File structure

```
src/
├── metrics.rs                  # NEW: init + graceful shutdown
├── config/mod.rs               # MODIFY: MetricsConfig, new secret schema
├── main.rs                     # MODIFY: call metrics::init
├── writer/
│   ├── mod.rs                  # MODIFY: intents counter, managed_hosts gauge; pass op label
│   └── retry_npm.rs            # MODIFY: add op label param, timer, retries counter
└── reconciler/mod.rs           # MODIFY: sweep_lag gauge; pass op label

tests/
└── metrics.rs                  # NEW: scrape-endpoint integration test

examples/
├── config.toml                 # MODIFY: new schema, [metrics] block
└── grafana-dashboard.json      # NEW

README.md                       # MODIFY: metrics + secrets sections
Cargo.toml                      # MODIFY: add metrics deps
```

---

## Task 1: Scaffold metrics module and add deps

**Files:**
- Modify: `Cargo.toml`, `src/lib.rs`
- Create: `src/metrics.rs` (stub)

- [ ] **Step 1: Resolve latest versions.** Use context7 for `metrics` and `metrics-exporter-prometheus`. Confirm the `http-listener` feature name is still current.

- [ ] **Step 2: Add deps to `Cargo.toml`.**
  ```toml
  metrics = "0.24"
  metrics-exporter-prometheus = { version = "0.17", default-features = false, features = ["http-listener"] }
  ```
  Update versions to whatever context7 reports as latest stable.

- [ ] **Step 3: Declare module in `src/lib.rs`.** Add `pub mod metrics;` in alphabetical order.

- [ ] **Step 4: Stub `src/metrics.rs`.**
  ```rust
  use crate::config::MetricsConfig;

  pub fn init(_cfg: &MetricsConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
      Ok(())
  }
  ```
  Real implementation comes in Task 3 after `MetricsConfig` exists.

- [ ] **Step 5: Verify.**
  ```
  cargo build
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```
  All pass (except `MetricsConfig` doesn't exist yet — Task 2 adds it, so this task will temporarily have a compile error that Task 2 resolves).

  Adjust: to keep the crate compiling after Task 1, use a placeholder local type:
  ```rust
  // temporary until config::MetricsConfig exists; Task 2 removes this
  pub struct MetricsConfig { pub enabled: bool }
  pub fn init(_cfg: &MetricsConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> { Ok(()) }
  ```

- [ ] **Step 6: Commit.**
  ```
  git add Cargo.toml Cargo.lock src/lib.rs src/metrics.rs
  git commit -m "chore: add metrics and prometheus exporter dependencies"
  ```

---

## Task 2: Config schema — MetricsConfig and inline-secret shape (TDD)

**Files:**
- Modify: `src/config/mod.rs`, `src/metrics.rs` (remove placeholder struct)

- [ ] **Step 1: Write failing tests.** Extend the existing `tests` module in `src/config/mod.rs`. Add to the existing `with_env` helper pattern. New tests:

  ```rust
  #[test]
  fn metrics_defaults_to_disabled() {
      let cfg = base_config();
      with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "tk")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert!(!r.config.metrics.enabled);
      });
  }

  #[test]
  fn password_literal_path() {
      let mut cfg = base_config();
      cfg.npm.password = Some("lit".into());
      cfg.npm.password_env = None;
      with_env(&[("CF_API_TOKEN", "tk")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert!(matches!(r.npm_credential, NpmCredential::EmailPassword { password, .. } if password == "lit"));
      });
  }

  #[test]
  fn password_env_path() {
      let mut cfg = base_config();
      cfg.npm.password_env = Some("SOMETHING_ELSE".into());
      with_env(&[("SOMETHING_ELSE", "env-val"), ("CF_API_TOKEN", "tk")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert!(matches!(r.npm_credential, NpmCredential::EmailPassword { password, .. } if password == "env-val"));
      });
  }

  #[test]
  fn password_defaults_to_npm_password_env() {
      let mut cfg = base_config();
      cfg.npm.password = None;
      cfg.npm.password_env = None;
      with_env(&[("NPM_PASSWORD", "default-env"), ("CF_API_TOKEN", "tk")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert!(matches!(r.npm_credential, NpmCredential::EmailPassword { password, .. } if password == "default-env"));
      });
  }

  #[test]
  fn password_and_password_env_both_set_fails() {
      let mut cfg = base_config();
      cfg.npm.password = Some("lit".into());
      cfg.npm.password_env = Some("NPM_PASSWORD".into());
      with_env(&[("NPM_PASSWORD", "env"), ("CF_API_TOKEN", "tk")], || {
          assert!(matches!(resolve_secrets(cfg.clone()), Err(ConfigError::Validation(_))));
      });
  }

  #[test]
  fn token_and_email_both_set_fails() {
      let mut cfg = base_config();
      cfg.npm.token_env = Some("NPM_TOKEN".into());
      // email is already Some("a@b") in base_config
      with_env(&[("NPM_TOKEN", "jwt"), ("CF_API_TOKEN", "tk"), ("NPM_PASSWORD", "pw")], || {
          assert!(matches!(resolve_secrets(cfg.clone()), Err(ConfigError::Validation(_))));
      });
  }

  #[test]
  fn token_literal_path() {
      let mut cfg = base_config();
      cfg.npm.email = None;
      cfg.npm.password_env = None;
      cfg.npm.token = Some("literal-jwt".into());
      with_env(&[("CF_API_TOKEN", "tk")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert!(matches!(r.npm_credential, NpmCredential::Token(t) if t == "literal-jwt"));
      });
  }

  #[test]
  fn api_token_literal_path() {
      let mut cfg = base_config();
      cfg.cloudflare.api_token = Some("cf-literal".into());
      cfg.cloudflare.api_token_env = None;
      with_env(&[("NPM_PASSWORD", "x")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert_eq!(r.cloudflare_global.as_deref(), Some("cf-literal"));
      });
  }

  #[test]
  fn api_token_and_env_both_set_fails() {
      let mut cfg = base_config();
      cfg.cloudflare.api_token = Some("lit".into());
      cfg.cloudflare.api_token_env = Some("CF_API_TOKEN".into());
      with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "tk")], || {
          assert!(matches!(resolve_secrets(cfg.clone()), Err(ConfigError::Validation(_))));
      });
  }

  #[test]
  fn domain_env_form_resolves() {
      let mut cfg = base_config();
      cfg.cloudflare.domains.insert("ex.com".into(), DomainToken::Env { env: "CF_TOKEN_EX".into() });
      with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "g"), ("CF_TOKEN_EX", "scoped")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert_eq!(r.cloudflare_per_domain.get("ex.com").unwrap(), "scoped");
      });
  }

  #[test]
  fn domain_token_form_resolves() {
      let mut cfg = base_config();
      cfg.cloudflare.domains.insert("ex.com".into(), DomainToken::Token { token: "literal".into() });
      with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "g")], || {
          let r = resolve_secrets(cfg.clone()).unwrap();
          assert_eq!(r.cloudflare_per_domain.get("ex.com").unwrap(), "literal");
      });
  }
  ```

  Also test parse-level rejection of old-shape domain strings via a `toml::from_str` case:

  ```rust
  #[test]
  fn old_bare_string_domain_shape_rejected() {
      let s = r#"
          [npm]
          url = "http://npm"
          email = "a@b"
          letsencrypt_email = "a@b"
          [cloudflare.domains]
          "example.com" = "CF_TOKEN_EX"
      "#;
      assert!(toml::from_str::<Config>(s).is_err());
  }
  ```

- [ ] **Step 2: Run tests.** Expect compile errors (new fields don't exist yet).

- [ ] **Step 3: Update the config schema.** Replace `NpmConfig`, `CloudflareConfig`, and add `MetricsConfig`:

  ```rust
  // top of file, alongside other Deserialize structs
  use std::net::SocketAddr;

  #[derive(Debug, Clone, Deserialize)]
  pub struct NpmConfig {
      pub url: String,
      pub email: Option<String>,
      pub password: Option<String>,
      pub password_env: Option<String>,
      pub token: Option<String>,
      pub token_env: Option<String>,
      pub letsencrypt_email: Option<String>,
  }

  #[derive(Debug, Clone, Default, Deserialize)]
  pub struct CloudflareConfig {
      pub api_token: Option<String>,
      pub api_token_env: Option<String>,
      #[serde(default)]
      pub domains: BTreeMap<String, DomainToken>,
  }

  #[derive(Debug, Clone, Deserialize)]
  #[serde(untagged)]
  pub enum DomainToken {
      Env { env: String },
      Token { token: String },
  }

  #[derive(Debug, Clone, Deserialize)]
  pub struct MetricsConfig {
      #[serde(default)]
      pub enabled: bool,
      #[serde(default = "default_metrics_bind")]
      pub bind: SocketAddr,
  }

  fn default_metrics_bind() -> SocketAddr {
      "0.0.0.0:9090".parse().unwrap()
  }

  impl Default for MetricsConfig {
      fn default() -> Self {
          Self { enabled: false, bind: default_metrics_bind() }
      }
  }
  ```

  Add `metrics` field to `Config`:
  ```rust
  #[serde(default)]
  pub metrics: MetricsConfig,
  ```

  Add `base_config()` updates so existing tests keep passing — initialize `password`, `password_env`, `token`, `token_env` to sensible defaults (all `None` except `password_env` when the old tests expect `NPM_PASSWORD`). Since our old tests set `cfg.npm.email = Some("a@b")` and relied on `NPM_PASSWORD`, set `password_env = Some("NPM_PASSWORD".into())` in `base_config` OR rely on the default-to-NPM_PASSWORD logic (see next step).

- [ ] **Step 4: Update `resolve_secrets`.** Use the `SecretSource` helper from the spec:

  ```rust
  enum SecretSource {
      Literal(String),
      EnvName(String),
  }

  impl SecretSource {
      fn resolve(self) -> Result<String, SecretError> {
          match self {
              SecretSource::Literal(s) => Ok(s),
              SecretSource::EnvName(name) => secrets::require(&name),
          }
      }

      fn from_pair(
          field: &str,
          inline: Option<String>,
          env_name: Option<String>,
          default_env: Option<&str>,
      ) -> Result<Option<Self>, ConfigError> {
          match (inline, env_name) {
              (Some(_), Some(_)) => Err(ConfigError::Validation(format!(
                  "{field}: set either {field} or {field}_env, not both"
              ))),
              (Some(v), None) => Ok(Some(SecretSource::Literal(v))),
              (None, Some(n)) => Ok(Some(SecretSource::EnvName(n))),
              (None, None) => Ok(default_env.map(|n| SecretSource::EnvName(n.to_string()))),
          }
      }
  }

  pub fn resolve_secrets(config: Config) -> Result<ResolvedConfig, ConfigError> {
      // Token and email are mutually exclusive.
      let has_token = config.npm.token.is_some() || config.npm.token_env.is_some();
      let has_email = config.npm.email.is_some();
      if has_token && has_email {
          return Err(ConfigError::Validation(
              "npm: set either email/password or token, not both".into(),
          ));
      }

      let npm_credential = if has_token {
          let source = SecretSource::from_pair(
              "npm.token",
              config.npm.token.clone(),
              config.npm.token_env.clone(),
              None,
          )?
          .ok_or_else(|| ConfigError::Validation("npm.token or npm.token_env required".into()))?;
          NpmCredential::Token(source.resolve()?)
      } else if let Some(email) = &config.npm.email {
          let source = SecretSource::from_pair(
              "npm.password",
              config.npm.password.clone(),
              config.npm.password_env.clone(),
              Some("NPM_PASSWORD"),
          )?
          .ok_or_else(|| ConfigError::Validation("npm.password or npm.password_env required".into()))?;
          NpmCredential::EmailPassword { email: email.clone(), password: source.resolve()? }
      } else {
          return Err(ConfigError::Validation(
              "npm.email (with password) or npm.token required".into(),
          ));
      };

      let cloudflare_global = match SecretSource::from_pair(
          "cloudflare.api_token",
          config.cloudflare.api_token.clone(),
          config.cloudflare.api_token_env.clone(),
          Some("CF_API_TOKEN"),
      )? {
          Some(s) => s.resolve().ok(),
          None => None,
      };

      let mut cloudflare_per_domain = BTreeMap::new();
      for (domain, entry) in &config.cloudflare.domains {
          let token = match entry {
              DomainToken::Env { env } => secrets::require(env)?,
              DomainToken::Token { token } => token.clone(),
          };
          cloudflare_per_domain.insert(domain.clone(), token);
      }

      if config.defaults.ssl
          && cloudflare_global.is_none()
          && cloudflare_per_domain.is_empty()
      {
          return Err(ConfigError::Validation(
              "defaults.ssl=true requires cloudflare.api_token(_env) or per-domain overrides".into(),
          ));
      }

      if config.defaults.ssl
          && config.npm.letsencrypt_email.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true)
      {
          return Err(ConfigError::Validation(
              "defaults.ssl=true requires npm.letsencrypt_email to be set and non-empty".into(),
          ));
      }

      Ok(ResolvedConfig {
          config,
          npm_credential,
          cloudflare_global,
          cloudflare_per_domain,
      })
  }
  ```

  Note: `cloudflare_global` swallows the resolution error when the `SecretSource::EnvName` path fails. Fine for an optional field — if it's required (defaults.ssl=true and no per-domain), the later check catches the missing-token case. But to match the old behavior where `CF_API_TOKEN` missing is silent, use `.ok()`.

- [ ] **Step 5: Remove the placeholder `MetricsConfig` from `src/metrics.rs`.** Update the stub to import from config:
  ```rust
  use crate::config::MetricsConfig;

  pub fn init(_cfg: &MetricsConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
      Ok(())
  }
  ```

- [ ] **Step 6: Run tests.** `cargo test --lib config` — all new tests pass. `cargo test --all-targets` — existing integration tests still pass (they don't touch the credential shape, just the writer/cache).

- [ ] **Step 7: Verify clippy/fmt.**
  ```
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```

- [ ] **Step 8: Commit.**
  ```
  git add src/config/mod.rs src/metrics.rs
  git commit -m "feat: inline-secret TOML schema and MetricsConfig"
  ```

---

## Task 3: Metrics init with live HTTP listener

**Files:**
- Modify: `src/metrics.rs`
- Create: `tests/metrics.rs`

- [ ] **Step 1: Write integration test.** Create `tests/metrics.rs`:

  ```rust
  use npm_docker_sync::config::MetricsConfig;

  #[tokio::test(flavor = "multi_thread")]
  async fn init_disabled_is_noop() {
      let cfg = MetricsConfig { enabled: false, ..MetricsConfig::default() };
      npm_docker_sync::metrics::init(&cfg).unwrap();
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn init_enabled_exposes_scrape_endpoint() {
      // Bind to an ephemeral port to avoid conflicts.
      let cfg = MetricsConfig {
          enabled: true,
          bind: "127.0.0.1:0".parse().unwrap(),
      };
      // Keep the guard alive for the duration of the test so the listener stays up.
      let _guard = npm_docker_sync::metrics::init(&cfg).unwrap();

      // Port 0 means "assign me a port" — but the exporter needs to surface it.
      // See the implementation step: init returns a MetricsHandle with `.bound_addr()`.
      let addr = _guard.as_ref().expect("enabled => Some").bound_addr();
      metrics::counter!("test_counter").increment(1);

      // Give the exporter a moment to record.
      tokio::time::sleep(std::time::Duration::from_millis(100)).await;

      let body = reqwest::get(format!("http://{addr}/metrics"))
          .await
          .unwrap()
          .text()
          .await
          .unwrap();
      assert!(body.contains("test_counter"), "scrape body: {body}");
  }
  ```

- [ ] **Step 2: Run — expect compile failure.** `cargo test --test metrics`.

- [ ] **Step 3: Implement `src/metrics.rs`.** Replace the stub:

  ```rust
  use std::net::SocketAddr;

  use metrics_exporter_prometheus::PrometheusBuilder;

  use crate::config::MetricsConfig;

  /// Handle returned from `init`. Dropping it shuts down the HTTP listener.
  pub struct MetricsHandle {
      bound: SocketAddr,
      // PrometheusHandle keeps the recorder alive; we don't expose it.
      _recorder: metrics_exporter_prometheus::PrometheusHandle,
  }

  impl MetricsHandle {
      pub fn bound_addr(&self) -> SocketAddr {
          self.bound
      }
  }

  pub fn init(cfg: &MetricsConfig) -> Result<Option<MetricsHandle>, Box<dyn std::error::Error + Send + Sync>> {
      if !cfg.enabled {
          return Ok(None);
      }
      // PrometheusBuilder's `with_http_listener` spawns the listener on a background task
      // when `install` is called. API may vary by version — verify via context7.
      let (recorder, exporter) = PrometheusBuilder::new()
          .with_http_listener(cfg.bind)
          .build()?;
      let handle = recorder.handle();
      metrics::set_global_recorder(recorder)
          .map_err(|e| format!("set_global_recorder: {e}"))?;

      // Bind the listener on a Tokio task.
      let bound = cfg.bind; // if `bind` was :0, the actual bound port is inside `exporter` — see below.
      tokio::spawn(async move {
          if let Err(e) = exporter.await {
              tracing::error!(error = %e, "metrics exporter exited");
          }
      });

      Ok(Some(MetricsHandle {
          bound,
          _recorder: handle,
      }))
  }
  ```

  **Adapt per 0.17 API:** the exporter may expose `bound_addr()` on the builder after binding. If the ephemeral-port case can't be recovered (common limitation), change the test to bind `127.0.0.1:<fixed-random-high-port>` computed once at test start. Acceptable fallback: use port `0` and read the bound addr by spawning the listener with the manual `with_http_listener_future` path if that API exists, otherwise fix the port.

  **Concrete fallback if `bound_addr` isn't exposed:** pick a fixed test port (e.g., based on `std::net::TcpListener::bind("127.0.0.1:0").local_addr()` before calling `init`, then pass that addr to `init`).

- [ ] **Step 4: Adjust test for ephemeral-port reality.** If the exporter doesn't expose `bound_addr`, rewrite the test prologue:

  ```rust
  // Grab an ephemeral port by binding + closing a socket.
  let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let port = probe.local_addr().unwrap().port();
  drop(probe);
  let cfg = MetricsConfig {
      enabled: true,
      bind: format!("127.0.0.1:{port}").parse().unwrap(),
  };
  let _guard = npm_docker_sync::metrics::init(&cfg).unwrap();
  // ... rest uses cfg.bind or `127.0.0.1:{port}`
  ```

  There's a TOCTOU race (something else could grab the port between drop and bind), but in practice this is reliable enough for tests.

- [ ] **Step 5: Run tests.** `cargo test --test metrics` — both pass.

- [ ] **Step 6: Verify.** Clippy + fmt clean.

- [ ] **Step 7: Commit.**
  ```
  git add src/metrics.rs tests/metrics.rs
  git commit -m "feat: initialize Prometheus metrics exporter"
  ```

---

## Task 4: Wire metrics init into main

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Call `metrics::init` after telemetry.** Between telemetry init and NPM connect:

  ```rust
  telemetry::init(&resolved.config.logging)
      .map_err(|e| anyhow::anyhow!("init telemetry: {e}"))?;
  let _metrics_guard = npm_docker_sync::metrics::init(&resolved.config.metrics)
      .map_err(|e| anyhow::anyhow!("init metrics: {e}"))?;
  if let Some(handle) = &_metrics_guard {
      tracing::info!(bind = %handle.bound_addr(), "metrics endpoint listening");
  }
  ```

  Use `use npm_docker_sync::{metrics, ...}` import. Hold `_metrics_guard` until the end of `main` so the listener stays up (drop semantics shut it down). The leading underscore keeps clippy's unused-variable lint quiet.

- [ ] **Step 2: Verify.**
  ```
  cargo build --release
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```

- [ ] **Step 3: Commit.**
  ```
  git add src/main.rs
  git commit -m "feat: wire metrics exporter into main"
  ```

---

## Task 5: Instrument retry wrapper

**Files:**
- Modify: `src/writer/retry_npm.rs`, `src/writer/mod.rs`, `src/reconciler/mod.rs`

This adds an `operation` label parameter to `with_retry`, a timer wrapping each inner call, and `retries_total` counter emits. Changes the `with_retry` signature, which touches every call site.

- [ ] **Step 1: Modify `with_retry` signature in `src/writer/retry_npm.rs`.**

  ```rust
  pub(crate) async fn with_retry<T, F, Fut>(
      npm: &NpmClient,
      operation: &'static str,
      mut f: F,
  ) -> Result<T, NpmError>
  where
      F: FnMut() -> Fut,
      Fut: std::future::Future<Output = Result<T, NpmError>>,
  {
      let policy = RetryPolicy::default();

      // Wrap f() in a timer-recording closure so each HTTP attempt is timed.
      let wrapped = || {
          let start = std::time::Instant::now();
          let fut = f();
          async move {
              let res = fut.await;
              let elapsed = start.elapsed().as_secs_f64();
              metrics::histogram!("npm_docker_sync_npm_request_duration_seconds", "operation" => operation)
                  .record(elapsed);
              res
          }
      };

      // Wrap classify so we can count retries.
      let classify = |e: &NpmError| {
          let kind = classify_npm(e);
          match kind {
              FailKind::Transient => {
                  metrics::counter!("npm_docker_sync_retries_total", "kind" => "transient").increment(1);
              }
              FailKind::Unauthorized => {
                  metrics::counter!("npm_docker_sync_retries_total", "kind" => "unauthorized").increment(1);
              }
              FailKind::NonTransient => {}
          }
          kind
      };

      let result = retry_call(
          &policy,
          classify,
          || async {
              if let Err(e) = npm.refresh_token().await {
                  tracing::warn!(error = %e, "token refresh failed during retry");
              }
          },
          wrapped,
      )
      .await;

      if let Err(RetryError::Exhausted(_)) = &result {
          metrics::counter!("npm_docker_sync_retries_total", "kind" => "exhausted").increment(1);
      }

      result.map_err(|re| match re {
          RetryError::Exhausted(e) | RetryError::NonTransient(e) => e,
      })
  }
  ```

  **Borrow note:** the closure `wrapped` moves `start` and `fut`, and `f()` happens outside the `async move`. That's fine — `f` is `FnMut`, so we can call it repeatedly. However, `retry_call` takes an `FnMut` bound, so passing a `|| { ... }` closure that itself captures `&mut f` works.

  If the borrow gets tangled, simplest fallback: don't wrap `f` — time each attempt by having the caller's closures include the timing code. Less DRY but straightforward. Try the wrapped approach first.

- [ ] **Step 2: Update all `with_retry` call sites.** Search: `grep -rn "with_retry" src`. Expect 8 sites in `src/writer/mod.rs` + 1 in `src/reconciler/mod.rs`. Each becomes:

  ```rust
  // Before
  let hosts = with_retry(&self.npm, || self.npm.list_proxy_hosts()).await?;
  // After
  let hosts = with_retry(&self.npm, "list", || self.npm.list_proxy_hosts()).await?;
  ```

  Operation labels:
  - `list_proxy_hosts` → `"list"`
  - `create_proxy_host` → `"create"`
  - `update_proxy_host` → `"update"`
  - `delete_proxy_host` → `"delete"`
  - `request_certificate` → `"cert"`

  Don't instrument login — it's called from inside `refresh_token` which is the `on_unauthorized` hook, not through `with_retry`.

- [ ] **Step 3: Update retry unit tests.** The tests in `src/writer/retry.rs` don't change — they test `retry_call` directly, not `with_retry`. If any test in `tests/writer_actor.rs` breaks due to changed behavior, adjust.

- [ ] **Step 4: Run tests.** `cargo test --all-targets` — all green.

- [ ] **Step 5: Verify.**
  ```
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  ```

- [ ] **Step 6: Commit.**
  ```
  git add src/writer/retry_npm.rs src/writer/mod.rs src/reconciler/mod.rs
  git commit -m "feat: instrument NPM retries and request latency"
  ```

---

## Task 6: Instrument writer (intents_total, managed_hosts)

**Files:**
- Modify: `src/writer/mod.rs`

- [ ] **Step 1: Update `NpmWriter::handle` to classify and record.** Wrap the match:

  ```rust
  async fn handle(&mut self, intent: Intent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
      let kind_label = match &intent { Intent::Upsert { .. } => "upsert", Intent::Remove { .. } => "remove" };
      let result = match intent {
          Intent::Upsert { container_id, spec } => self.upsert(&container_id, &spec).await,
          Intent::Remove { container_id } => self.remove(&container_id).await,
      };
      let result_label = match &result { Ok(_) => "ok", Err(_) => "error" };
      metrics::counter!(
          "npm_docker_sync_intents_total",
          "kind" => kind_label,
          "result" => result_label,
      ).increment(1);
      result
  }
  ```

  Note: "skipped" (the conflict-skip path and cleanup-disabled path) returns `Ok(())` today. To capture that as a distinct label, `upsert` and `remove` would need to return an enum or a richer Result. Pragmatic solution: accept the slight loss of fidelity (conflicts are logged at `error` level, so they're visible in logs), and count "skipped" as "ok" for the metric. Document this in a comment.

- [ ] **Step 2: Update `managed_hosts` gauge** — record after each cache mutation in `upsert` and `remove`:

  ```rust
  // After: self.cache.insert(container_id.to_string(), h.id);
  metrics::gauge!("npm_docker_sync_managed_hosts").set(self.cache.len() as f64);

  // Also after: self.cache.insert(container_id.to_string(), created.id);
  // and after: self.cache.remove(container_id);
  ```

  Also record after the initial cache-seed in `run()`:

  ```rust
  metrics::gauge!("npm_docker_sync_managed_hosts").set(self.cache.len() as f64);
  ```

- [ ] **Step 3: Run tests.** `cargo test --all-targets`. All pass.

- [ ] **Step 4: Verify.** Clippy + fmt clean.

- [ ] **Step 5: Commit.**
  ```
  git add src/writer/mod.rs
  git commit -m "feat: instrument writer intent outcomes and managed hosts"
  ```

---

## Task 7: Instrument reconciler (sweep_lag)

**Files:**
- Modify: `src/reconciler/mod.rs`

- [ ] **Step 1: Add `last_sweep_success: Arc<Mutex<Instant>>` to `Reconciler`.** Or, simpler, `tokio::sync::Mutex<Instant>` so it's cancel-safe around awaits. Initialize to `Instant::now()` in `new()`.

  Actually simpler still: since the reconciler task is single-threaded (runs inside one `tokio::spawn`), we can use `Instant` directly on `&mut self`. But `run` consumes `self`, so a field is fine.

  ```rust
  pub struct Reconciler {
      pub docker: DockerClient,
      pub npm: NpmClient,
      pub defaults: labels::Defaults,
      pub interval: Duration,
      pub tx: mpsc::Sender<Intent>,
      pub cancel: CancellationToken,
      // NEW (not pub — internal state):
      last_sweep_success: std::time::Instant,
  }
  ```

  Since all fields were `pub`, adding a private field breaks construction at every call site. **Alternative that avoids touching callers:** move `last_sweep_success` inside `run` as a local variable:

  ```rust
  pub async fn run(self) {
      let mut last_sweep_success = std::time::Instant::now();
      // ... existing sweep logic, pass &mut last_sweep_success to sweep()
  }
  ```

  Use that.

- [ ] **Step 2: Record lag before each sweep and update on success.**

  ```rust
  async fn sweep(
      &self,
      last_sweep_success: &mut std::time::Instant,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
      let lag = last_sweep_success.elapsed().as_secs_f64();
      metrics::gauge!("npm_docker_sync_reconciler_sweep_lag_seconds").set(lag);

      // ... existing sweep body ...

      *last_sweep_success = std::time::Instant::now();
      metrics::gauge!("npm_docker_sync_reconciler_sweep_lag_seconds").set(0.0);
      Ok(())
  }
  ```

  Call sites in `run` pass `&mut last_sweep_success`.

- [ ] **Step 3: Run tests.** `cargo test --all-targets`.

- [ ] **Step 4: Verify.** Clippy + fmt clean.

- [ ] **Step 5: Commit.**
  ```
  git add src/reconciler/mod.rs
  git commit -m "feat: instrument reconciler sweep lag"
  ```

---

## Task 8: Update example config

**Files:**
- Modify: `examples/config.toml`

- [ ] **Step 1: Rewrite the `[npm]` section** for the new secret schema (see spec). Add commented alternatives.

- [ ] **Step 2: Rewrite the `[cloudflare]` section** with `api_token` / `api_token_env` and the tagged per-domain form.

- [ ] **Step 3: Add the `[metrics]` section** (present by default, `enabled = false`).

- [ ] **Step 4: Commit.**
  ```
  git add examples/config.toml
  git commit -m "docs: update example config for new schema and metrics"
  ```

---

## Task 9: Add Grafana dashboard JSON

**Files:**
- Create: `examples/grafana-dashboard.json`

- [ ] **Step 1: Build the dashboard.** Use Grafana's UI against a local Prometheus scraping npm-docker-sync, add the five panels from the spec, export JSON via Dashboard → Share → Export → Save to file. Drop the file into `examples/`.

  **Alternative if no Grafana is available:** hand-write a minimal JSON using Grafana 10+ schema. Minimum viable structure:

  ```json
  {
    "title": "npm-docker-sync",
    "schemaVersion": 39,
    "version": 1,
    "refresh": "30s",
    "time": { "from": "now-1h", "to": "now" },
    "panels": [
      { "id": 1, "type": "timeseries", "title": "Intent rate",
        "targets": [{ "expr": "rate(npm_docker_sync_intents_total[5m])", "legendFormat": "{{result}}" }],
        "gridPos": { "h": 8, "w": 12, "x": 0, "y": 0 } },
      { "id": 2, "type": "stat", "title": "Managed hosts",
        "targets": [{ "expr": "npm_docker_sync_managed_hosts" }],
        "gridPos": { "h": 8, "w": 6, "x": 12, "y": 0 } },
      { "id": 3, "type": "timeseries", "title": "NPM latency (p50/p95/p99)",
        "targets": [
          { "expr": "histogram_quantile(0.50, rate(npm_docker_sync_npm_request_duration_seconds_bucket[5m]))", "legendFormat": "p50" },
          { "expr": "histogram_quantile(0.95, rate(npm_docker_sync_npm_request_duration_seconds_bucket[5m]))", "legendFormat": "p95" },
          { "expr": "histogram_quantile(0.99, rate(npm_docker_sync_npm_request_duration_seconds_bucket[5m]))", "legendFormat": "p99" }
        ],
        "gridPos": { "h": 8, "w": 12, "x": 0, "y": 8 } },
      { "id": 4, "type": "timeseries", "title": "Retry rate",
        "targets": [{ "expr": "rate(npm_docker_sync_retries_total[5m])", "legendFormat": "{{kind}}" }],
        "gridPos": { "h": 8, "w": 12, "x": 12, "y": 8 } },
      { "id": 5, "type": "timeseries", "title": "Reconciler sweep lag",
        "targets": [{ "expr": "npm_docker_sync_reconciler_sweep_lag_seconds" }],
        "gridPos": { "h": 8, "w": 12, "x": 0, "y": 16 } }
    ]
  }
  ```

  If the hand-written version imports cleanly into Grafana 10+ but lacks polish, that's acceptable — users can tweak.

- [ ] **Step 2: Validate JSON** with `python3 -m json.tool < examples/grafana-dashboard.json > /dev/null`.

- [ ] **Step 3: Commit.**
  ```
  git add examples/grafana-dashboard.json
  git commit -m "docs: add starter Grafana dashboard"
  ```

---

## Task 10: README updates

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add "Metrics" section.** Under a top-level heading, briefly:
  - How to enable (flip `enabled = true` in `[metrics]`)
  - Default bind
  - List of metric names and types
  - "Import `examples/grafana-dashboard.json` into Grafana for a starter dashboard."

- [ ] **Step 2: Add "Secrets in the config file" section.**
  - Two sources per field (`field = "literal"` vs `field_env = "NAME"`)
  - Recommendation: `*_env` for production, inline for single-host/dev
  - "TOML names the source" rule — env vars never override inline literals
  - Warning: inline-secret TOML must not be committed; bind-mount read-only; file mode 0600

- [ ] **Step 3: Update the existing labels/config section** if it shows an old `[cloudflare.domains]` shape.

- [ ] **Step 4: Commit.**
  ```
  git add README.md
  git commit -m "docs: document metrics and inline secrets"
  ```

---

## Task 11: Final verification

- [ ] **Step 1: Full test run.**
  ```
  cargo test --all-targets
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  cargo build --release
  ```
  All pass.

- [ ] **Step 2: Manual smoke (optional).** If Docker is available:
  - `docker build -f docker/Dockerfile -t npm-docker-sync:metrics-test .`
  - Run with a config that enables metrics
  - `curl http://localhost:9090/metrics` — should return Prometheus text format with the five metric families

- [ ] **Step 3: Summarize** the branch state — list of commits, test count, any remaining TODOs.

---

## Execution notes

- No `Co-Authored-By` trailers.
- Specific-file `git add` throughout.
- Conventional commit prefixes on every commit.
- All work on `feature/metrics-and-inline-secrets`. Merging into `develop` is a separate user-driven step.
- `metrics` crate macros are no-ops when no recorder is installed — instrumentation code left in place when metrics are disabled has zero runtime cost.

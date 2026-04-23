# Metrics and Inline-Secrets Design

**Date:** 2026-04-22
**Status:** Proposed

## Overview

Two independent features added on top of the v0.1 core service:

1. **Prometheus metrics endpoint** — an opt-in HTTP listener exposing a small, operationally useful set of metrics about intents processed, NPM API latency, retry behavior, and reconciler health. A minimal Grafana dashboard is shipped alongside.
2. **Secrets-in-TOML opt-in** — inline secret values in the config file as an alternative to env-var references, for single-host deployments. Each secret field declares its source explicitly; there is no ambient env-var override of inline literals.

Both features are opt-in. Users who do nothing see identical behavior to the v0.1 release.

## Goals

- Observability: operators can scrape Prometheus and tell at a glance whether the service is healthy and doing its job.
- Deployment flexibility: single-host users don't have to thread env vars through their stack when a TOML file is already mounted read-only.
- Zero regression: existing unit and integration tests stay green without modification.
- Minimal schema churn beyond the inline-secrets change (which is a pre-release breaking change, accepted).

## Non-goals

- High-cardinality metrics (per-domain labels).
- Auth/TLS on the metrics endpoint. Deployments that need that can put it behind a reverse proxy.
- Hot config reload. Secrets are read once at startup.
- Secret encryption at rest in the TOML file. If inline secrets are used, operators are responsible for file permissions and bind-mount hygiene.

---

## Feature 1: Prometheus metrics

### Stack

- [`metrics`](https://docs.rs/metrics) — instrumentation facade. Macros compile to no-ops when no recorder is installed, so disabled metrics have zero runtime cost.
- [`metrics-exporter-prometheus`](https://docs.rs/metrics-exporter-prometheus) — Prometheus text-format exporter with a built-in `hyper` HTTP listener. No additional HTTP framework needed.

### Config

A new optional `[metrics]` section:

```toml
[metrics]
enabled = false              # default; flip to true to enable
bind = "0.0.0.0:9090"         # default; change to e.g. "127.0.0.1:9090" to restrict
```

Schema (in `src/config/mod.rs`):

```rust
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

Added to `Config`: `#[serde(default)] pub metrics: MetricsConfig`. When the section is omitted entirely, `enabled` defaults to false and bind takes the default value, which matches "disabled" behavior.

The example config ships with the block present and `enabled = false`, so the feature is discoverable without hunting through the README.

### Metrics exposed

| Name | Type | Labels | Description |
|------|------|--------|-------------|
| `npm_docker_sync_intents_total` | counter | `kind` (`upsert`/`remove`), `result` (`ok`/`error`/`skipped`) | Intents processed by the writer |
| `npm_docker_sync_managed_hosts` | gauge | — | Current count of proxy hosts owned by this service |
| `npm_docker_sync_npm_request_duration_seconds` | histogram | `operation` (`list`/`create`/`update`/`delete`/`cert`/`login`) | NPM API call latency |
| `npm_docker_sync_retries_total` | counter | `kind` (`transient`/`unauthorized`/`exhausted`) | Retry-wrapper events |
| `npm_docker_sync_reconciler_sweep_lag_seconds` | gauge | — | Seconds since the last successful reconciler sweep completed |

All histograms use the exporter's default buckets. No per-domain labels (explicit non-goal — cardinality protection).

### Instrumentation points

- **Writer** (`src/writer/mod.rs`): at the end of `handle()`, classify the outcome and increment `intents_total{kind,result}`. When the cache is mutated (insert on upsert success, remove on delete success), update `managed_hosts` to `cache.len()`.
- **Retry wrapper** (`src/writer/retry_npm.rs`): the timer wraps each individual invocation of the inner closure `f()` inside `retry_call`, so every HTTP attempt is timed separately (including the ones that failed and were retried). Record `npm_request_duration_seconds{operation=<op>}`. The `operation` label is a static string supplied by the caller (`with_retry(&npm, "list", || npm.list_proxy_hosts())`). Increment `retries_total{kind}` whenever a transient/unauthorized/exhausted classification occurs in the retry loop.
- **Reconciler** (`src/reconciler/mod.rs`): track `last_sweep_success: Instant` in the `Reconciler` struct. Before each sweep, record `reconciler_sweep_lag_seconds = now - last_sweep_success`. On successful sweep completion, update `last_sweep_success = now`.

### Module layout

New file: `src/metrics.rs`

```rust
use crate::config::MetricsConfig;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

pub fn init(cfg: &MetricsConfig) -> Result<Option<PrometheusHandle>, Box<dyn std::error::Error + Send + Sync>> {
    if !cfg.enabled {
        return Ok(None);
    }
    let builder = PrometheusBuilder::new().with_http_listener(cfg.bind);
    let handle = builder.install_recorder()?;
    // The HTTP listener task is spawned by install(); install_recorder returns a handle for introspection.
    Ok(Some(handle))
}
```

Main wires it after telemetry init and before task spawns:

```rust
let _metrics_handle = metrics::init(&resolved.config.metrics)?;
```

The handle is held for the program lifetime (dropping it would shut the exporter down).

### Grafana dashboard

Shipped at `examples/grafana-dashboard.json`. Five panels, one per metric:

1. **Intent rate** — stacked graph of `rate(npm_docker_sync_intents_total[5m])` by `result`
2. **Managed hosts** — single stat showing current `npm_docker_sync_managed_hosts`
3. **NPM latency** — graph with p50, p95, p99 quantiles from `npm_docker_sync_npm_request_duration_seconds`, broken down by `operation`
4. **Retry rate** — graph of `rate(npm_docker_sync_retries_total[5m])` by `kind`
5. **Reconciler lag** — graph of `npm_docker_sync_reconciler_sweep_lag_seconds`

Deliberately plain: no dashboard variables, no alerting rules, no thresholds. Easy to import and extend.

### Testing

- `metrics::init` with `enabled = false` returns `Ok(None)`.
- `metrics::init` with `enabled = true` and a `127.0.0.1:0` bind succeeds; a subsequent HTTP GET to the exporter's address returns 200 with Prometheus text format. (Uses an ephemeral port by binding to `:0`. We'll need to expose the actual bound port — `PrometheusBuilder` provides this via its returned future, so the test waits on connection instead of port reflection.)
- Invalid bind (e.g., IP already taken, unparseable) returns `Err`.

---

## Feature 2: Secrets-in-TOML opt-in

### Paradigm: TOML names the source of truth

Each secret field declares exactly one source. Env vars do not override inline literals. There is no ambient crossover.

- `field = "literal"` → use the literal. Env vars are ignored.
- `field_env = "ENV_VAR_NAME"` → use whatever env var `ENV_VAR_NAME` holds. Env unset is a hard error.
- Neither set → fall back to a built-in default `field_env = "<CANONICAL_NAME>"` for backwards-friendly behavior.

This paradigm removes ambient-env surprises. Secret rotation still works:
- Production deployments using `field_env` rotate by changing the env var (standard 12-factor).
- Single-host deployments using `field = "literal"` rotate by editing the TOML file.

### Schema changes

#### `[npm]` — NPM credentials

```toml
[npm]
url = "http://npm:81"
email = "admin@example.com"

# Password (pick one, or omit both to default to password_env = "NPM_PASSWORD"):
password = "hunter2"
# password_env = "NPM_PASSWORD"

# Token auth — mutually exclusive with email/password:
# token = "eyJ..."
# token_env = "NPM_TOKEN"

letsencrypt_email = "ops@example.com"
```

#### `[cloudflare]` — global CF token

```toml
[cloudflare]
# Pick one, or omit both to default to api_token_env = "CF_API_TOKEN":
api_token = "cf_literal"
# api_token_env = "CF_API_TOKEN"
```

#### `[cloudflare.domains]` — per-domain (breaking schema change)

Old shape (bare string values, v0.1 pre-release):

```toml
[cloudflare.domains]
"example.com" = "CF_TOKEN_EX"
```

New shape (tagged values):

```toml
[cloudflare.domains]
"example.com" = { env = "CF_TOKEN_EX" }
"other.com"   = { token = "cf_literal_token_here" }
```

The service has not been released, so breaking this shape is acceptable. If TOML parsing sees the old bare-string form, the error message explicitly points at the new tagged form.

### Validation rules (fail fast with `ConfigError::Validation`)

1. `email` set together with `token` or `token_env` → error ("use email/password OR token, not both").
2. Both `password` and `password_env` set → error.
3. Both `token` and `token_env` set → error.
4. Both `api_token` and `api_token_env` set → error.
5. A per-domain entry with both `env` and `token` set → error.
6. A per-domain entry with neither `env` nor `token` set → error.
7. When using email/password and neither `password` nor `password_env` is set → default to `password_env = "NPM_PASSWORD"`.
8. When neither `api_token` nor `api_token_env` is set and `defaults.ssl = true` and no per-domain overrides have a usable token → fail (existing check, updated).
9. Any `*_env` pointing at an unset env var → `SecretError::Missing(var_name)`.

### Internal representation

`ResolvedConfig` retains its existing shape: `NpmCredential { EmailPassword{email, password} | Token(String) }`, `cloudflare_global: Option<String>`, `cloudflare_per_domain: BTreeMap<String, String>`. Downstream code (writer, main) sees only resolved literal strings; it never learns whether a secret came from TOML or env.

### `SecretSource` helper

A new private enum inside `src/config/mod.rs`:

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
        inline: Option<&str>,
        env_name: Option<&str>,
        default_env: Option<&str>,
    ) -> Result<Option<Self>, ConfigError> {
        match (inline, env_name) {
            (Some(_), Some(_)) => Err(ConfigError::Validation(format!(
                "{field}: set either {field} or {field}_env, not both"
            ))),
            (Some(v), None) => Ok(Some(SecretSource::Literal(v.to_string()))),
            (None, Some(n)) => Ok(Some(SecretSource::EnvName(n.to_string()))),
            (None, None) => Ok(default_env.map(|n| SecretSource::EnvName(n.to_string()))),
        }
    }
}
```

Call sites in `resolve_secrets` become one line each:

```rust
let password = SecretSource::from_pair(
    "npm.password",
    config.npm.password.as_deref(),
    config.npm.password_env.as_deref(),
    Some("NPM_PASSWORD"),
)?
.map(SecretSource::resolve)
.transpose()?;
```

### Testing

Extend `src/config/mod.rs` tests:

- `password_literal_happy_path` — `password = "x"`, no env needed.
- `password_env_happy_path` — `password_env = "NPM_PASSWORD"`, env set.
- `password_defaults_to_env_npm_password` — neither set, env set.
- `password_and_password_env_both_set_fails` — validation error.
- `token_and_email_together_fails` — validation error.
- `api_token_literal_happy_path` — and its `_env` sibling.
- `domains_tagged_env_form_parses` — `{ env = "X" }` tagged form.
- `domains_tagged_token_form_parses` — `{ token = "..." }` tagged form.
- `domains_both_env_and_token_fails` — `{ env = "X", token = "Y" }` validation error.
- `domains_bare_string_form_errors_with_migration_hint` — the old `"example.com" = "STR"` shape produces a parse error whose message points at the new shape. (This depends on toml/serde producing a reasonable error; if the error is too generic, we add a post-parse check that re-reads the raw table and emits a better message.)

No wiremock or integration test changes: `ResolvedConfig`'s public shape is unchanged.

### Documentation

- **`examples/config.toml`** — populate the new fields with comments explaining each source option.
- **README** — new section "Secrets in the config file":
  - One-paragraph explanation of the two sources per field.
  - Recommend `*_env` for production deployments; inline for single-host/dev.
  - Document the "TOML names the source" rule — env vars do not override inline literals.
  - Warn: inline-secret TOML must not be committed to git; bind-mount read-only in containers; file mode `0600` on the host.

---

## Dependencies

Add to `[dependencies]`:

```toml
metrics = "0.24"
metrics-exporter-prometheus = { version = "0.17", default-features = false, features = ["http-listener"] }
```

Verify latest stable versions via context7 at implementation time.

## Rollout

- Metrics default disabled. Existing deployments see zero change.
- Secrets schema is breaking for per-domain overrides only. Since the service has never been released, the impact is the project's own examples/config.toml — which this design updates.
- One new feature branch: `feature/metrics-and-inline-secrets`, off `develop`.

## Open questions

None. All decisions resolved in brainstorming.

## Future work (still deferred)

- SECURITY.md and documented token rotation procedure.
- Forward-host override label, advanced nginx config label, access-list support.

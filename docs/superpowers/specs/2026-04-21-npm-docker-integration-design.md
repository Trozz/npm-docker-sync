# NPM Docker Integration — Design

**Date:** 2026-04-21
**Status:** Proposed

## Overview

A Rust service that watches Docker containers for routing labels and keeps a Nginx Proxy Manager (NPM) instance in sync with the running containers. When a container starts carrying `nginx_proxy_url=app.example.com`, the service creates a corresponding proxy host in NPM and requests a Let's Encrypt certificate via Cloudflare's DNS-01 challenge. When the container stops, the matching proxy host is removed, but only for hosts the service owns.

## Goals

- Zero-touch proxy-host management for Docker-based deployments.
- Safe by default: never modify user-created proxy hosts.
- Resilient to transient failures (network blips, NPM restarts, event loss).
- Stateless across restarts: authoritative state lives in NPM itself.

## Non-goals

- Multi-tenant or multi-NPM support.
- Reverse proxies other than NPM (no Traefik/Caddy abstraction).
- Manual UI. Configuration is Docker labels plus a TOML file only.

## Architecture

Single Rust binary running a tokio runtime with three long-lived tasks that communicate over a single `mpsc::channel<Intent>`:

```
Docker watcher ──┐
                 ├──▶ mpsc ──▶ NPM writer ──▶ NPM API
Reconciler ──────┘
```

- **Docker watcher** streams Docker events and emits `Intent::Upsert` or `Intent::Remove` as containers appear and vanish.
- **Reconciler** fires on a configurable tick, lists Docker + NPM state, and emits the same intents to close drift.
- **NPM writer** is the sole consumer of the channel and the sole caller of the NPM API. It owns retry policy, idempotency checks, and the in-memory cache.

This shape eliminates races between the event stream and the reconciler without any explicit locking. Both producers emit the same message type, and a single consumer serializes all writes.

## Components

### `config`
Loads TOML, overlays env-var secrets, validates on startup. Fails fast on missing credentials.

### `docker`
Wraps `bollard`. Responsibilities: event subscription, `inspect` calls, label parsing, port selection.

Port-selection rules:
1. If the `nginx_proxy_port` label is present, use it.
2. Otherwise, if the container exposes exactly one port, use that port.
3. Otherwise, pick the lowest-numbered exposed port and log a warning.

### `npm`
Typed REST client for NPM: login, proxy-host CRUD, Let's Encrypt certificate request. Handles JWT refresh on 401 transparently. Stamps an ownership marker into the `meta` field on every host it creates or updates.

### `writer`
The actor. Drains the intent channel, reconciles against its in-memory cache and NPM state, calls the NPM client. Core decision logic (`diff_spec`) is a pure function over in-memory types and is unit-tested in isolation.

### `reconciler`
Periodic sweep. Lists labeled containers and NPM hosts, emits intents for drift. The planning logic (`reconciler::plan`) is a pure function.

### `main`
Wires config, clients, channel, cancellation token. Spawns the three tasks. Waits for SIGTERM, cancels, joins.

## Data flow

### New container
1. `docker run -l nginx_proxy_url=app.example.com -l nginx_proxy_port=3000 myapp`
2. Watcher receives `start`, inspects, emits `Intent::Upsert { container_id, spec }`.
3. Writer cache miss triggers `npm.create_proxy_host(spec, marker)` followed by `npm.request_lets_encrypt_cert(...)`.
4. Cache is updated with `container_id → host_id`.

### Container removed
1. Watcher receives `die` or `destroy`, emits `Intent::Remove { container_id }`.
2. Writer consults cache and the NPM ownership marker.
3. If `cleanup.on_remove = true` (default) and the marker belongs to this service, the host is deleted.
4. Otherwise log and skip.

### Reconciliation
Every `interval_seconds` (default 300):
1. List labeled Docker containers.
2. List NPM proxy hosts.
3. For every container, emit `Upsert`. The writer's diff short-circuits if nothing changed.
4. For every owned NPM host whose container is missing, emit `Remove`.

### Startup
1. Load config.
2. Writer logs into NPM. Retries on failure, continues in degraded state if exhausted.
3. Reconciler fires one synchronous sweep to seed the cache and heal prior drift.
4. Watcher starts streaming events.
5. Reconciler's periodic ticker begins.

### Edge cases
- **Label change without restart.** Docker emits no event. Caught on the next reconciliation sweep. Accepted trade-off.
- **Two containers claiming the same URL.** First-writer-wins. The second is logged as a conflict and skipped. The writer never hijacks a host it does not own.
- **NPM unreachable at startup.** Writer retries login. Service stays up in degraded mode. Reconciler sweeps harmlessly until NPM returns.

## Ownership marker

Every proxy host the service creates carries a marker in NPM's `meta` JSON field:

```json
{
  "npm_docker_sync": {
    "version": 1,
    "container_id": "7b3f...",
    "container_name": "myapp",
    "managed_at": "2026-04-21T12:34:56Z"
  }
}
```

Rules:
- The writer refuses to modify or delete any host whose `meta` lacks the marker. User-managed hosts are invisible to the service.
- On update, the writer preserves all other keys in `meta`. It only touches `npm_docker_sync`.
- On conflict (a host exists for a domain but the marker points at a different `container_id`), the writer logs and skips.

This makes the service genuinely stateless. Wipe the cache, restart the binary, and the first reconciliation sweep rebuilds everything from NPM.

## Configuration

TOML file (`/etc/npm-docker-sync/config.toml` by default, `--config` flag to override). Secrets come from env vars only, never from TOML.

```toml
[npm]
url = "http://nginx-proxy-manager:81"
email = "admin@example.com"
# password from NPM_PASSWORD env var
# alternatively:
# token_env = "NPM_TOKEN"

[docker]
socket = "/var/run/docker.sock"

[forward_host]
strategy = "container_name"   # container_name | container_ip | host_port
host_address = "host.docker.internal"
network = "proxy"

[reconciler]
interval_seconds = 300

[cleanup]
on_remove = true

[cloudflare]
# global token from CF_API_TOKEN env var

[cloudflare.domains]
"example.com" = "CF_TOKEN_EXAMPLE"
"other.com"   = "CF_TOKEN_OTHER"

[defaults]
scheme = "http"
ssl = true
websockets = true
block_exploits = true

[logging]
level = "info"
format = "json"
```

Required environment variables:
- `NPM_PASSWORD`, or the env var named by `[npm].token_env`.
- `CF_API_TOKEN`, unless every domain has an override in `[cloudflare.domains]`.
- Every env var name referenced in `[cloudflare.domains]`.

### Docker labels
- `nginx_proxy_url` (required): public hostname.
- `nginx_proxy_port`: target port. Inferred if omitted.
- `nginx_proxy_scheme`: `http` or `https`. Default from `[defaults]`.
- `nginx_proxy_ssl`: `true` or `false`.
- `nginx_proxy_websockets`: `true` or `false`.
- `nginx_proxy_block_exploits`: `true` or `false`.

## Error handling and retries

All NPM calls go through a single retry wrapper.
- 3 attempts total, exponential backoff (1s, 4s) with jitter.
- Transient: network errors, timeouts, HTTP 5xx, HTTP 429.
- Non-transient: HTTP 4xx. Exception: 401 triggers one re-login and retries the original request without counting against the attempt budget.
- Exhaustion logs an `error` with identifying context. The next reconciliation sweep re-emits the intent, so nothing is permanently lost.

Docker socket disconnects trigger reconnection with backoff (1s, 4s, 16s, capped at 60s, indefinite). Events missed during the outage are caught by the next reconciliation sweep.

Parse and config errors on labels produce a `warn` log, then skip. No retry.

## Testing strategy

### Unit tests
- `config`: TOML fixtures plus env-var scenarios.
- `docker::labels`: label parsing and port-selection rules.
- `npm::meta`: ownership-marker encode, decode, and merge-with-existing preservation.
- `writer::diff`: `diff_spec` pure function.
- `reconciler::plan`: intent-planning pure function.

### Integration tests
- NPM client against `wiremock`: login plus 401 refresh, retry-on-5xx, no-retry-on-4xx, ownership preservation on update.
- Docker client against real Docker via `testcontainers-rs` in CI.

### End-to-end
One `docker compose` smoke test: NPM plus the service plus two labeled containers. Assert proxy hosts exist with correct upstreams.

### Not tested
- Actual Let's Encrypt issuance (external, rate-limited).
- Cloudflare API (the service never calls it directly, NPM does).

## Project layout

Rust binary crate:

```
src/
├── main.rs
├── lib.rs
├── config/
├── docker/        # mod.rs, watcher.rs, labels.rs, spec.rs
├── npm/           # mod.rs, auth.rs, proxy_hosts.rs, certificates.rs, meta.rs, types.rs
├── writer/        # mod.rs, diff.rs, retry.rs
├── reconciler/    # mod.rs, plan.rs
├── intent.rs
├── cloudflare.rs
└── telemetry.rs

tests/             # wiremock + testcontainers
examples/
docker/            # Dockerfile + compose example
docs/
.github/workflows/
```

Key dependencies (latest stable): `tokio`, `bollard`, `reqwest`, `serde`, `toml` (or `figment`), `tracing`, `tracing-subscriber`, `thiserror`, `anyhow`, `tokio-util`. Dev-dependencies: `wiremock`, `testcontainers`, `pretty_assertions`.

Docker image: multi-stage `rust:1-slim` build stage, `gcr.io/distroless/cc-debian12` final stage. Runs as non-root. Reads config from `/etc/npm-docker-sync/config.toml`. Mounts the Docker socket or accepts `DOCKER_HOST`.

Gitflow per user convention: `main`, `develop`, `feature/*`.

## Open questions

None. All design decisions resolved during brainstorming.

## Future work (not in v1)

- `nginx_proxy_forward_host` label to override the auto-detected upstream.
- `nginx_proxy_advanced_config` label for custom nginx snippets.
- NPM access-list support.
- Prometheus metrics endpoint.
- Opt-in for secrets in TOML (for simpler single-host deployments).

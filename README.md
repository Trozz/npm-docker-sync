# npm-docker-sync

Watches Docker container labels and automatically creates, updates, and deletes proxy hosts in [Nginx Proxy Manager][npm].

## What it does

- Monitors containers for the `nginx_proxy_url` label and manages corresponding NPM proxy hosts.
- Creates or updates a proxy host when a labeled container starts; removes it when the container is removed (configurable).
- Issues Let's Encrypt certificates via Cloudflare DNS-01 challenge when SSL is requested.
- Marks every host it creates with an `npm_docker_sync` ownership marker in NPM's `meta` field; hosts without that marker are never touched.
- Runs a periodic reconciliation sweep (default: every 300 s) to correct any drift between running containers and NPM state.

## Quick start

1. Create a `.env` file with your credentials:

   ```
   NPM_PASSWORD=your-npm-password
   CF_API_TOKEN=your-cloudflare-global-token
   ```

2. Copy and edit the example config:

   ```sh
   cp examples/config.toml /etc/npm-docker-sync/config.toml
   # Edit npm.email, npm.letsencrypt_email, and any defaults you want to change.
   ```

3. Start the stack:

   ```sh
   docker compose -f docker/docker-compose.example.yml up -d
   ```

4. Run a labeled container:

   ```sh
   docker run -d -l nginx_proxy_url=app.example.com nginx
   ```

   A proxy host for `app.example.com` will appear in NPM within seconds.

## Docker labels reference

| Label | Required | Default | Description |
|---|---|---|---|
| `nginx_proxy_url` | yes | | Public hostname for the proxy host. |
| `nginx_proxy_port` | no | inferred | Target port. If the container exposes one port, that port is used. If it exposes multiple, the lowest is used. |
| `nginx_proxy_scheme` | no | `http` | Forwarding scheme: `http` or `https`. |
| `nginx_proxy_ssl` | no | from config | When `true`, requests a Let's Encrypt cert via Cloudflare DNS-01. |
| `nginx_proxy_websockets` | no | from config | Enable WebSocket support on the proxy host. |
| `nginx_proxy_block_exploits` | no | from config | Enable NPM's block-exploits option. |

## Configuration

[`examples/config.toml`][config] is the annotated reference for all options.

Environment variables used at runtime:

| Variable | Purpose |
|---|---|
| `NPM_PASSWORD` | Password for NPM email/password authentication. |
| `CF_API_TOKEN` | Global Cloudflare API token, used for all domains unless overridden. |
| `CF_TOKEN_*` | Per-domain Cloudflare tokens. Map a domain to the variable name under `[cloudflare.domains]` in the config. |

## How forwarding works

Three strategies control how NPM is told to reach a container:

- `container_name` (default): NPM forwards to the container by name. Requires NPM and the target container to share a Docker network.
- `container_ip`: Resolves the container's IP on the named network and forwards to that address.
- `host_port`: Forwards to a configurable host address plus the container's published port. Suitable when NPM and the target run on separate networks or hosts.

Set `forward_host.strategy` in the config to choose. See [`examples/config.toml`][config] for the related options (`network`, `host_address`).

## Ownership and safety

Every proxy host this service creates carries an `npm_docker_sync` marker stored in NPM's `meta` field. The service only ever reads, updates, or deletes hosts that carry this marker. Any proxy host you create manually through the NPM UI is invisible to npm-docker-sync and will never be modified or removed. Cleanup on container removal is enabled by default and can be disabled by setting `cleanup.on_remove = false` in the config.

## Testing

```sh
# Unit tests + wiremock integration tests (no external dependencies)
cargo test

# End-to-end smoke test (requires a running Docker daemon and internet access to pull images)
cargo test --test e2e_smoke -- --ignored
```

The smoke test boots a real Nginx Proxy Manager container, spawns the compiled binary as a subprocess, starts a labeled `nginx:alpine` container, and asserts that NPM creates and then removes the corresponding proxy host. It is opt-in (`#[ignore]`) and skipped by CI.

## Building locally

Requires Rust stable 1.85 or later, plus `pkg-config` and `cmake` (needed by `aws-lc-rs`).

```sh
cargo build --release
```

The compiled binary is written to `target/release/npm-docker-sync`.

## License

MIT

[npm]: https://nginxproxymanager.com
[config]: examples/config.toml

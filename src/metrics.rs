use std::net::SocketAddr;

use metrics_exporter_prometheus::PrometheusBuilder;

use crate::config::MetricsConfig;

/// Guard returned from `init`. Holding it keeps intent clear at the call site.
/// The global recorder and HTTP listener remain active for the process lifetime
/// regardless of when this guard is dropped.
pub struct MetricsGuard {
    bound: SocketAddr,
}

impl MetricsGuard {
    pub fn bound_addr(&self) -> SocketAddr {
        self.bound
    }
}

pub fn init(
    cfg: &MetricsConfig,
) -> Result<Option<MetricsGuard>, Box<dyn std::error::Error + Send + Sync>> {
    if !cfg.enabled {
        return Ok(None);
    }

    PrometheusBuilder::new()
        .with_http_listener(cfg.bind)
        .install()?;

    Ok(Some(MetricsGuard { bound: cfg.bind }))
}

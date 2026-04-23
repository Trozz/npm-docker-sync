use crate::config::MetricsConfig;

pub fn init(_cfg: &MetricsConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
}

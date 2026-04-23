// Temporary placeholder until `config::MetricsConfig` exists (Task 2).
#[allow(dead_code)]
pub struct MetricsConfig {
    pub enabled: bool,
}

pub fn init(_cfg: &MetricsConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
}

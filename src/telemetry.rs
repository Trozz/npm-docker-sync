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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_does_not_panic() {
        // Calling init more than once in a test binary returns Err after the first success;
        // we only care that it doesn't panic.
        let _ = init(&LoggingConfig::default());
    }
}

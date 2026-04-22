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
pub struct DockerConfig {
    pub socket: String,
}
impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            socket: "/var/run/docker.sock".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForwardHostConfig {
    pub strategy: ForwardStrategy,
    pub host_address: Option<String>,
    pub network: Option<String>,
}
impl Default for ForwardHostConfig {
    fn default() -> Self {
        Self {
            strategy: ForwardStrategy::ContainerName,
            host_address: None,
            network: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForwardStrategy {
    ContainerName,
    ContainerIp,
    HostPort,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReconcilerConfig {
    pub interval_seconds: u64,
}
impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            interval_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CleanupConfig {
    pub on_remove: bool,
}
impl Default for CleanupConfig {
    fn default() -> Self {
        Self { on_remove: true }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudflareConfig {
    #[serde(default)]
    pub domains: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Defaults {
    pub scheme: Scheme,
    pub ssl: bool,
    pub websockets: bool,
    pub block_exploits: bool,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            scheme: Scheme::Http,
            ssl: true,
            websockets: true,
            block_exploits: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
}
impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub config: Config,
    pub npm_credential: NpmCredential,
    pub cloudflare_global: Option<String>,
    pub cloudflare_per_domain: BTreeMap<String, String>,
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
    let npm_credential = if let Some(env_name) = &config.npm.token_env {
        NpmCredential::Token(secrets::require(env_name)?)
    } else if let Some(email) = &config.npm.email {
        let password = secrets::require("NPM_PASSWORD")?;
        NpmCredential::EmailPassword {
            email: email.clone(),
            password,
        }
    } else {
        return Err(ConfigError::Validation(
            "npm.email or npm.token_env required".into(),
        ));
    };

    let cloudflare_global = secrets::optional("CF_API_TOKEN");
    let mut cloudflare_per_domain = BTreeMap::new();
    for (domain, env_name) in &config.cloudflare.domains {
        cloudflare_per_domain.insert(domain.clone(), secrets::require(env_name)?);
    }
    if config.defaults.ssl && cloudflare_global.is_none() && cloudflare_per_domain.is_empty() {
        return Err(ConfigError::Validation(
            "defaults.ssl=true requires CF_API_TOKEN or per-domain cloudflare.domains".into(),
        ));
    }
    Ok(ResolvedConfig {
        config,
        npm_credential,
        cloudflare_global,
        cloudflare_per_domain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<R>(pairs: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
        let _g = ENV_LOCK.lock().unwrap();
        let prior: Vec<(String, Option<String>)> = pairs
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(*k).ok()))
            .collect();
        for (k, v) in pairs {
            unsafe { std::env::set_var(k, v) };
        }
        let out = f();
        for (k, v) in &prior {
            match v {
                Some(x) => unsafe { std::env::set_var(k, x) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        out
    }

    fn base_config() -> Config {
        Config {
            npm: NpmConfig {
                url: "http://npm".into(),
                email: Some("a@b".into()),
                token_env: None,
                letsencrypt_email: Some("a@b".into()),
            },
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
        with_env(&[], || {
            unsafe { std::env::remove_var("NPM_PASSWORD") };
            let r = resolve_secrets(cfg.clone());
            assert!(matches!(
                r,
                Err(ConfigError::Secret(SecretError::Missing(_)))
            ));
        });
    }

    #[test]
    fn email_password_happy_path() {
        let cfg = base_config();
        with_env(
            &[("NPM_PASSWORD", "hunter2"), ("CF_API_TOKEN", "tk")],
            || {
                let r = resolve_secrets(cfg.clone()).unwrap();
                assert!(matches!(
                    r.npm_credential,
                    NpmCredential::EmailPassword { .. }
                ));
                assert_eq!(r.cloudflare_global.as_deref(), Some("tk"));
            },
        );
    }

    #[test]
    fn per_domain_cf_env_vars_resolved() {
        let mut cfg = base_config();
        cfg.cloudflare
            .domains
            .insert("example.com".into(), "CF_TOKEN_EX".into());
        with_env(
            &[
                ("NPM_PASSWORD", "x"),
                ("CF_API_TOKEN", "global"),
                ("CF_TOKEN_EX", "scoped"),
            ],
            || {
                let r = resolve_secrets(cfg.clone()).unwrap();
                assert_eq!(
                    r.cloudflare_per_domain.get("example.com").unwrap(),
                    "scoped"
                );
            },
        );
    }

    #[test]
    fn token_credential_happy_path() {
        let mut cfg = base_config();
        cfg.npm.email = None;
        cfg.npm.token_env = Some("NPM_TOKEN".into());
        with_env(&[("NPM_TOKEN", "jwt"), ("CF_API_TOKEN", "x")], || {
            let r = resolve_secrets(cfg.clone()).unwrap();
            assert!(matches!(r.npm_credential, NpmCredential::Token(_)));
        });
    }

    #[test]
    fn ssl_default_with_no_cf_token_fails_validation() {
        let cfg = base_config();
        with_env(&[("NPM_PASSWORD", "x")], || {
            unsafe { std::env::remove_var("CF_API_TOKEN") };
            assert!(matches!(
                resolve_secrets(cfg.clone()),
                Err(ConfigError::Validation(_))
            ));
        });
    }
}

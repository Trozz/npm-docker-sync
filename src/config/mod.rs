pub mod secrets;

use self::secrets::SecretError;
use crate::docker::spec::Scheme;
use serde::Deserialize;
use std::{collections::BTreeMap, net::SocketAddr, path::Path};

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
    #[serde(default)]
    pub metrics: MetricsConfig,
}

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
        Self {
            enabled: false,
            bind: default_metrics_bind(),
        }
    }
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

pub fn load(path: &Path) -> Result<ResolvedConfig, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&text)?;
    resolve_secrets(config)
}

pub fn resolve_secrets(config: Config) -> Result<ResolvedConfig, ConfigError> {
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
        .ok_or_else(|| {
            ConfigError::Validation("npm.password or npm.password_env required".into())
        })?;
        NpmCredential::EmailPassword {
            email: email.clone(),
            password: source.resolve()?,
        }
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

    if config.defaults.ssl && cloudflare_global.is_none() && cloudflare_per_domain.is_empty() {
        return Err(ConfigError::Validation(
            "defaults.ssl=true requires cloudflare.api_token(_env) or per-domain overrides".into(),
        ));
    }

    if config.defaults.ssl
        && config
            .npm
            .letsencrypt_email
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
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
                password: None,
                password_env: Some("NPM_PASSWORD".into()),
                token: None,
                token_env: None,
                letsencrypt_email: Some("a@b".into()),
            },
            docker: DockerConfig::default(),
            forward_host: ForwardHostConfig::default(),
            reconciler: ReconcilerConfig::default(),
            cleanup: CleanupConfig::default(),
            cloudflare: CloudflareConfig {
                api_token_env: Some("CF_API_TOKEN".into()),
                ..CloudflareConfig::default()
            },
            defaults: Defaults::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
        }
    }

    #[test]
    fn email_password_requires_npm_password() {
        let mut cfg = base_config();
        cfg.npm.password_env = None;
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
        cfg.cloudflare.domains.insert(
            "example.com".into(),
            DomainToken::Env {
                env: "CF_TOKEN_EX".into(),
            },
        );
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
        cfg.npm.password_env = None;
        cfg.npm.token_env = Some("NPM_TOKEN".into());
        with_env(&[("NPM_TOKEN", "jwt"), ("CF_API_TOKEN", "x")], || {
            let r = resolve_secrets(cfg.clone()).unwrap();
            assert!(matches!(r.npm_credential, NpmCredential::Token(_)));
        });
    }

    #[test]
    fn ssl_default_with_no_cf_token_fails_validation() {
        let mut cfg = base_config();
        cfg.cloudflare.api_token_env = None;
        with_env(&[("NPM_PASSWORD", "x")], || {
            unsafe { std::env::remove_var("CF_API_TOKEN") };
            assert!(matches!(
                resolve_secrets(cfg.clone()),
                Err(ConfigError::Validation(_))
            ));
        });
    }

    #[test]
    fn ssl_default_with_no_letsencrypt_email_fails_validation() {
        let mut cfg = base_config();
        cfg.npm.letsencrypt_email = None;
        with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "tk")], || {
            assert!(matches!(
                resolve_secrets(cfg.clone()),
                Err(ConfigError::Validation(_))
            ));
        });
    }

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
            assert!(matches!(
                r.npm_credential,
                NpmCredential::EmailPassword { password, .. } if password == "lit"
            ));
        });
    }

    #[test]
    fn password_env_path() {
        let mut cfg = base_config();
        cfg.npm.password_env = Some("SOMETHING_ELSE".into());
        with_env(
            &[("SOMETHING_ELSE", "env-val"), ("CF_API_TOKEN", "tk")],
            || {
                let r = resolve_secrets(cfg.clone()).unwrap();
                assert!(matches!(
                    r.npm_credential,
                    NpmCredential::EmailPassword { password, .. } if password == "env-val"
                ));
            },
        );
    }

    #[test]
    fn password_defaults_to_npm_password_env() {
        let mut cfg = base_config();
        cfg.npm.password = None;
        cfg.npm.password_env = None;
        with_env(
            &[("NPM_PASSWORD", "default-env"), ("CF_API_TOKEN", "tk")],
            || {
                let r = resolve_secrets(cfg.clone()).unwrap();
                assert!(matches!(
                    r.npm_credential,
                    NpmCredential::EmailPassword { password, .. } if password == "default-env"
                ));
            },
        );
    }

    #[test]
    fn password_and_password_env_both_set_fails() {
        let mut cfg = base_config();
        cfg.npm.password = Some("lit".into());
        cfg.npm.password_env = Some("NPM_PASSWORD".into());
        with_env(&[("NPM_PASSWORD", "env"), ("CF_API_TOKEN", "tk")], || {
            assert!(matches!(
                resolve_secrets(cfg.clone()),
                Err(ConfigError::Validation(_))
            ));
        });
    }

    #[test]
    fn token_and_email_both_set_fails() {
        let mut cfg = base_config();
        cfg.npm.token_env = Some("NPM_TOKEN".into());
        with_env(
            &[
                ("NPM_TOKEN", "jwt"),
                ("CF_API_TOKEN", "tk"),
                ("NPM_PASSWORD", "pw"),
            ],
            || {
                assert!(matches!(
                    resolve_secrets(cfg.clone()),
                    Err(ConfigError::Validation(_))
                ));
            },
        );
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
            unsafe { std::env::remove_var("CF_API_TOKEN") };
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
            assert!(matches!(
                resolve_secrets(cfg.clone()),
                Err(ConfigError::Validation(_))
            ));
        });
    }

    #[test]
    fn domain_env_form_resolves() {
        let mut cfg = base_config();
        cfg.cloudflare.domains.insert(
            "ex.com".into(),
            DomainToken::Env {
                env: "CF_TOKEN_EX".into(),
            },
        );
        with_env(
            &[
                ("NPM_PASSWORD", "x"),
                ("CF_API_TOKEN", "g"),
                ("CF_TOKEN_EX", "scoped"),
            ],
            || {
                let r = resolve_secrets(cfg.clone()).unwrap();
                assert_eq!(r.cloudflare_per_domain.get("ex.com").unwrap(), "scoped");
            },
        );
    }

    #[test]
    fn domain_token_form_resolves() {
        let mut cfg = base_config();
        cfg.cloudflare.domains.insert(
            "ex.com".into(),
            DomainToken::Token {
                token: "literal".into(),
            },
        );
        with_env(&[("NPM_PASSWORD", "x"), ("CF_API_TOKEN", "g")], || {
            let r = resolve_secrets(cfg.clone()).unwrap();
            assert_eq!(r.cloudflare_per_domain.get("ex.com").unwrap(), "literal");
        });
    }

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
}

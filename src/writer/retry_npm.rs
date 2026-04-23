use crate::npm::NpmClient;
use crate::npm::NpmError;
use crate::npm::auth::AuthError;
use crate::npm::certificates::CertError;
use crate::npm::proxy_hosts::ProxyHostError;
use crate::writer::retry::{FailKind, RetryError, RetryPolicy, retry_call};

pub(crate) fn classify_npm(e: &NpmError) -> FailKind {
    let status = match e {
        NpmError::Auth(AuthError::Status(s)) => Some(*s),
        NpmError::ProxyHost(ProxyHostError::Status(s)) => Some(*s),
        NpmError::Cert(CertError::Status(s)) => Some(*s),
        _ => None,
    };
    match status {
        Some(401) => FailKind::Unauthorized,
        Some(s) if (400..500).contains(&s) => FailKind::NonTransient,
        Some(s) if s >= 500 || s == 429 => FailKind::Transient,
        _ => {
            let is_transport = matches!(
                e,
                NpmError::Auth(AuthError::Http(_))
                    | NpmError::ProxyHost(ProxyHostError::Http(_))
                    | NpmError::Cert(CertError::Http(_))
            );
            if is_transport {
                FailKind::Transient
            } else {
                FailKind::NonTransient
            }
        }
    }
}

pub(crate) async fn with_retry<T, F, Fut>(npm: &NpmClient, f: F) -> Result<T, NpmError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, NpmError>>,
{
    let policy = RetryPolicy::default();
    retry_call(
        &policy,
        classify_npm,
        || async {
            if let Err(e) = npm.refresh_token().await {
                tracing::warn!(error = %e, "token refresh failed during retry");
            }
        },
        f,
    )
    .await
    .map_err(|re| match re {
        RetryError::Exhausted(e) | RetryError::NonTransient(e) => e,
    })
}

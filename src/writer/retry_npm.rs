use crate::npm::auth::AuthError;
use crate::npm::certificates::CertError;
use crate::npm::proxy_hosts::ProxyHostError;
use crate::npm::{NpmClient, NpmError};
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

pub(crate) async fn with_retry<T, F, Fut>(
    npm: &NpmClient,
    operation: &'static str,
    mut f: F,
) -> Result<T, NpmError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, NpmError>>,
{
    let policy = RetryPolicy::default();

    // Time each individual HTTP attempt. `wrapped` captures `f` mutably;
    // each call starts a fresh timer and records duration when the future resolves.
    let mut wrapped = || {
        let fut = f();
        async move {
            let start = std::time::Instant::now();
            let res = fut.await;
            let elapsed = start.elapsed().as_secs_f64();
            metrics::histogram!(
                "npm_docker_sync_npm_request_duration_seconds",
                "operation" => operation
            )
            .record(elapsed);
            res
        }
    };

    // Intercept the classifier so we can count retry events.
    let classify = |e: &NpmError| {
        let kind = classify_npm(e);
        match kind {
            FailKind::Transient => {
                metrics::counter!(
                    "npm_docker_sync_retries_total",
                    "kind" => "transient"
                )
                .increment(1);
            }
            FailKind::Unauthorized => {
                metrics::counter!(
                    "npm_docker_sync_retries_total",
                    "kind" => "unauthorized"
                )
                .increment(1);
            }
            FailKind::NonTransient => {}
        }
        kind
    };

    let result = retry_call(
        &policy,
        classify,
        || async {
            if let Err(e) = npm.refresh_token().await {
                tracing::warn!(error = %e, "token refresh failed during retry");
            }
        },
        &mut wrapped,
    )
    .await;

    if let Err(RetryError::Exhausted(_)) = &result {
        metrics::counter!(
            "npm_docker_sync_retries_total",
            "kind" => "exhausted"
        )
        .increment(1);
    }

    result.map_err(|re| match re {
        RetryError::Exhausted(e) | RetryError::NonTransient(e) => e,
    })
}

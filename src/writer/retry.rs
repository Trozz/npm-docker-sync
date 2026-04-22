use std::time::Duration;

use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    Transient,
    NonTransient,
    Unauthorized,
}

pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RetryError<E> {
    #[error("exhausted retries: {0}")]
    Exhausted(E),
    #[error("non-transient: {0}")]
    NonTransient(E),
}

pub async fn retry_call<F, Fut, T, E, Classify, OnUnauth, FutU>(
    policy: &RetryPolicy,
    classify: Classify,
    mut on_unauthorized: OnUnauth,
    mut f: F,
) -> Result<T, RetryError<E>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    Classify: Fn(&E) -> FailKind,
    OnUnauth: FnMut() -> FutU,
    FutU: std::future::Future<Output = ()>,
    E: std::fmt::Display,
{
    let mut attempt: u32 = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => match classify(&e) {
                FailKind::NonTransient => return Err(RetryError::NonTransient(e)),
                FailKind::Unauthorized => {
                    on_unauthorized().await;
                    // Don't consume attempt budget on 401.
                    continue;
                }
                FailKind::Transient => {
                    attempt += 1;
                    if attempt >= policy.max_attempts {
                        return Err(RetryError::Exhausted(e));
                    }
                    let base_secs = policy.base.as_secs_f64();
                    let backoff = base_secs * 4.0_f64.powi((attempt - 1) as i32);
                    let jitter = rand::rng().random_range(0.0..0.3);
                    tokio::time::sleep(Duration::from_secs_f64(backoff + jitter)).await;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    async fn succeeds_on_first_attempt() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let res: Result<i32, RetryError<&str>> = retry_call(
            &RetryPolicy::default(),
            |_| FailKind::Transient,
            || async {},
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &str>(42)
                }
            },
        )
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_on_transient_then_gives_up() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let res: Result<i32, RetryError<&str>> = retry_call(
            &RetryPolicy::default(),
            |_| FailKind::Transient,
            || async {},
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("boom")
                }
            },
        )
        .await;
        assert!(matches!(res, Err(RetryError::Exhausted(_))));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_retry_non_transient() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let res: Result<i32, RetryError<&str>> = retry_call(
            &RetryPolicy::default(),
            |_| FailKind::NonTransient,
            || async {},
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("no")
                }
            },
        )
        .await;
        assert!(matches!(res, Err(RetryError::NonTransient(_))));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn unauthorized_refreshes_without_burning_attempts() {
        let call_count = Arc::new(AtomicU32::new(0));
        let refresh_count = Arc::new(AtomicU32::new(0));
        let cc = call_count.clone();
        let rc = refresh_count.clone();
        let res: Result<i32, RetryError<&str>> = retry_call(
            &RetryPolicy::default(),
            |e| {
                if *e == "401" {
                    FailKind::Unauthorized
                } else {
                    FailKind::NonTransient
                }
            },
            || {
                let rc = rc.clone();
                async move {
                    rc.fetch_add(1, Ordering::SeqCst);
                }
            },
            || {
                let cc = cc.clone();
                async move {
                    let n = cc.fetch_add(1, Ordering::SeqCst);
                    if n == 0 { Err("401") } else { Ok(7) }
                }
            },
        )
        .await;
        assert_eq!(res.unwrap(), 7);
        assert_eq!(refresh_count.load(Ordering::SeqCst), 1);
    }
}

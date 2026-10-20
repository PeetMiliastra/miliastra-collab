//! Retries temporary storage-provider errors (429, 5xx, timeouts) a few times instead
//! of failing on the first one.
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use std::time::Duration;

const ATTEMPTS: u32 = 4;
const MAX_WAIT: Duration = Duration::from_secs(10);

/// Requests that can be repeated without creating something twice.
fn idempotent(method: &Method) -> bool {
    [
        Method::GET,
        Method::HEAD,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ]
    .contains(method)
}

/// A 429 means the request wasn't processed, so anything can retry. After a server error
/// or timeout the work may already be done, so only idempotent requests retry.
fn should_retry(method: &Method, outcome: &reqwest::Result<Response>) -> bool {
    match outcome {
        Ok(r) if r.status() == StatusCode::TOO_MANY_REQUESTS => true,
        Ok(r) => r.status().is_server_error() && idempotent(method),
        Err(e) if e.is_connect() => true,
        Err(e) => e.is_timeout() && idempotent(method),
    }
}

/// Uses the provider's Retry-After if given, otherwise waits 1, 2 and 4 seconds.
/// Returns `None` if the provider wants a long wait; the caller gets the response instead.
fn delay(outcome: &reqwest::Result<Response>, attempt: u32) -> Option<Duration> {
    let requested = outcome
        .as_ref()
        .ok()
        .and_then(|r| r.headers().get(reqwest::header::RETRY_AFTER))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    match requested {
        Some(wait) if wait > MAX_WAIT => None,
        Some(wait) => Some(wait),
        None => Some(Duration::from_secs(1 << attempt)),
    }
}

pub(crate) trait SendRetrying {
    async fn send_retrying(self) -> reqwest::Result<Response>;
}
impl SendRetrying for RequestBuilder {
    async fn send_retrying(self) -> reqwest::Result<Response> {
        let mut attempt = 0;
        loop {
            // A streaming body can't be cloned, so that request is only sent once.
            let Some(next) = self.try_clone() else {
                return self.send().await;
            };
            let (client, request) = next.build_split();
            let request = request?;
            let method = request.method().clone();
            let outcome = client.execute(request).await;
            attempt += 1;
            if attempt >= ATTEMPTS || !should_retry(&method, &outcome) {
                return outcome;
            }
            let Some(wait) = delay(&outcome, attempt - 1) else {
                return outcome;
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode as Code, routing::any, Router};
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };
    async fn server(fail: u32, status: Code) -> (String, Arc<AtomicU32>) {
        let hits = Arc::new(AtomicU32::new(0));
        let counter = hits.clone();
        let app = Router::new().route(
            "/",
            any(move || {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < fail {
                        (status, [("Retry-After", "0")], "busy")
                    } else {
                        (Code::OK, [("Retry-After", "0")], "ok")
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, hits)
    }
    #[tokio::test]
    async fn temporary_gateway_errors_are_retried_for_reads() {
        let (url, hits) = server(2, Code::GATEWAY_TIMEOUT).await;
        let r = reqwest::Client::new()
            .get(&url)
            .send_retrying()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }
    #[tokio::test]
    async fn posts_retry_rate_limits_but_not_server_errors() {
        let (url, hits) = server(1, Code::TOO_MANY_REQUESTS).await;
        let r = reqwest::Client::new()
            .post(&url)
            .body("x")
            .send_retrying()
            .await
            .unwrap();
        assert_eq!(
            (r.status(), hits.load(Ordering::SeqCst)),
            (StatusCode::OK, 2)
        );
        let (url, hits) = server(1, Code::GATEWAY_TIMEOUT).await;
        let r = reqwest::Client::new()
            .post(&url)
            .body("x")
            .send_retrying()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a POST may already have taken effect"
        );
    }
    #[test]
    fn long_provider_delays_are_left_to_the_caller() {
        let response = |retry_after: &str| {
            Ok(Response::from(
                axum::http::Response::builder()
                    .status(429)
                    .header("Retry-After", retry_after)
                    .body("")
                    .unwrap(),
            ))
        };
        assert_eq!(delay(&response("3"), 0), Some(Duration::from_secs(3)));
        assert_eq!(delay(&response("120"), 0), None);
    }
    #[tokio::test]
    async fn persistent_failures_stop_after_a_few_attempts() {
        let (url, hits) = server(100, Code::SERVICE_UNAVAILABLE).await;
        let r = reqwest::Client::new()
            .get(&url)
            .send_retrying()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(hits.load(Ordering::SeqCst), ATTEMPTS);
    }
}

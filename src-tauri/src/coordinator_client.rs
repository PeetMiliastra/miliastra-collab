//! Requests to the coordinator. Never carries stage files, keys or storage tokens.
use anyhow::Result;
use miliastra_core::errors::{ErrorCode, ServiceError};
use std::{collections::BTreeSet, sync::Mutex};
pub const VERSION_MISMATCH_MESSAGE: &str = "Update required: this app and the collaboration service use incompatible versions. Update the app, or ask the project owner to update the service.";
pub struct CoordinatorClient {
    http: reqwest::Client,
    mismatches: Mutex<BTreeSet<(String, String)>>,
}
impl CoordinatorClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            mismatches: Mutex::new(BTreeSet::new()),
        })
    }
    pub async fn request(
        &self,
        endpoint: &str,
        project: &str,
        token: String,
        method: reqwest::Method,
        route: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let mut req = self
            .http
            .request(method, format!("{endpoint}/projects/{project}{route}"))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await?;
        let status = response.status();
        let compatible = response
            .headers()
            .get("x-miliastra-contract")
            .and_then(|value| value.to_str().ok())
            == Some(miliastra_core::contract_fingerprint());
        if compatible {
            // Even an authentication error proves a previously mismatched server was updated.
            self.set_protocol_mismatch(endpoint, project, false);
        }
        if !status.is_success() {
            let value: serde_json::Value = response.json().await.unwrap_or_default();
            let code = value
                .get("code")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or(match status.as_u16() {
                    401 => ErrorCode::Unauthorized,
                    404 => ErrorCode::NotFound,
                    429 => ErrorCode::RateLimited,
                    500..=599 => ErrorCode::Unavailable,
                    _ => ErrorCode::Conflict,
                });
            if code == ErrorCode::ProtocolMismatch {
                self.set_protocol_mismatch(endpoint, project, true);
            }
            return Err(ServiceError::new(
                code,
                value["error"]
                    .as_str()
                    .unwrap_or("The collaboration service is unavailable"),
            )
            .into());
        }
        if !compatible {
            self.set_protocol_mismatch(endpoint, project, true);
            return Err(
                ServiceError::new(ErrorCode::ProtocolMismatch, VERSION_MISMATCH_MESSAGE).into(),
            );
        }
        Ok(response.json().await?)
    }
    fn set_protocol_mismatch(&self, endpoint: &str, project: &str, mismatch: bool) {
        let key = (
            endpoint.trim_end_matches('/').to_owned(),
            project.to_owned(),
        );
        let mut mismatches = self.mismatches.lock().unwrap();
        if mismatch {
            mismatches.insert(key);
        } else {
            mismatches.remove(&key);
        }
    }
    pub fn protocol_mismatch(&self, endpoint: &str, project: &str) -> bool {
        self.mismatches.lock().unwrap().contains(&(
            endpoint.trim_end_matches('/').to_owned(),
            project.to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn response(
        status: axum::http::StatusCode,
        body: serde_json::Value,
        contract: &str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        responses(vec![(status, body, contract.to_owned())]).await
    }
    async fn responses(
        replies: Vec<(axum::http::StatusCode, serde_json::Value, String)>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{response::IntoResponse, routing::get};
        let replies = std::sync::Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
        let router = axum::Router::new().route(
            "/projects/p",
            get(move || {
                let (status, body, contract) = replies.lock().unwrap().pop_front().unwrap();
                async move {
                    (
                        status,
                        [
                            ("x-miliastra-contract", contract),
                            ("connection", "close".into()),
                        ],
                        axum::Json(body),
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (endpoint, task)
    }
    #[tokio::test]
    async fn error_codes_are_independent_of_display_wording() {
        let (ep, task) = response(
            axum::http::StatusCode::CONFLICT,
            serde_json::json!({"code":"reservationLost","error":"Localized explanatory text"}),
            miliastra_core::contract_fingerprint(),
        )
        .await;
        let error = CoordinatorClient::new()
            .unwrap()
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .unwrap_err();
        task.abort();
        assert_eq!(
            error.downcast_ref::<ServiceError>().unwrap().code,
            ErrorCode::ReservationLost
        );
    }
    #[tokio::test]
    async fn mismatched_or_missing_contracts_are_rejected() {
        for contract in ["wrong", ""] {
            let (ep, task) = response(
                axum::http::StatusCode::OK,
                serde_json::json!({"ok":true}),
                contract,
            )
            .await;
            let client = CoordinatorClient::new().unwrap();
            let error = client
                .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
                .await
                .unwrap_err();
            assert!(client.protocol_mismatch(&ep, "p"));
            assert!(client.protocol_mismatch(&format!("{ep}/"), "p"));
            assert!(!client.protocol_mismatch(&ep, "different-project"));
            assert!(!client.protocol_mismatch("https://different-service.example", "p"));
            assert!(error.to_string().contains("Update required"));
            task.abort();
            assert_eq!(
                error.downcast_ref::<ServiceError>().unwrap().code,
                ErrorCode::ProtocolMismatch
            );
        }
    }
    #[tokio::test]
    async fn matching_contract_returns_typed_json() {
        let (ep, task) = response(
            axum::http::StatusCode::OK,
            serde_json::json!({"ok":true}),
            miliastra_core::contract_fingerprint(),
        )
        .await;
        let result = CoordinatorClient::new()
            .unwrap()
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .unwrap();
        task.abort();
        assert_eq!(result["ok"], true);
    }
    #[tokio::test]
    async fn matching_response_clears_mismatch_even_when_authentication_fails() {
        use axum::http::StatusCode;
        for status in [StatusCode::OK, StatusCode::UNAUTHORIZED] {
            let (ep, task) = responses(vec![
                (
                    StatusCode::OK,
                    serde_json::json!({"ok":true}),
                    "old-contract".into(),
                ),
                (
                    status,
                    serde_json::json!({"code":"unauthorized","error":"Sign in again"}),
                    miliastra_core::contract_fingerprint().into(),
                ),
            ])
            .await;
            let client = CoordinatorClient::new().unwrap();
            assert!(client
                .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
                .await
                .is_err());
            assert!(client.protocol_mismatch(&ep, "p"));
            let result = client
                .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
                .await;
            assert_eq!(result.is_ok(), status.is_success());
            assert!(!client.protocol_mismatch(&ep, "p"));
            task.abort();
        }
    }
    #[tokio::test]
    async fn structured_mismatch_errors_are_kept_for_the_connection_notice() {
        let (ep, task) = response(
            axum::http::StatusCode::CONFLICT,
            serde_json::json!({"code":"protocolMismatch","error":"Localized update guidance"}),
            miliastra_core::contract_fingerprint(),
        )
        .await;
        let client = CoordinatorClient::new().unwrap();
        let error = client
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>().unwrap().code,
            ErrorCode::ProtocolMismatch
        );
        assert!(client.protocol_mismatch(&ep, "p"));
        task.abort();
    }
    #[tokio::test]
    async fn network_failure_does_not_erase_a_known_version_mismatch() {
        let (ep, task) = response(
            axum::http::StatusCode::OK,
            serde_json::json!({"ok":true}),
            "old-contract",
        )
        .await;
        let client = CoordinatorClient::new().unwrap();
        assert!(client
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .is_err());
        task.abort();
        let _ = task.await;
        assert!(client
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .is_err());
        assert!(client.protocol_mismatch(&ep, "p"));
        let fresh = CoordinatorClient::new().unwrap();
        assert!(fresh
            .request(&ep, "p", "token".into(), reqwest::Method::GET, "", None)
            .await
            .is_err());
        assert!(!fresh.protocol_mismatch(&ep, "p"));
    }
}

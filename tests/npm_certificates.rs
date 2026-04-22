use npm_docker_sync::npm::certificates::{CertificatesClient, LetsEncryptRequest};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn request_creates_cert_and_returns_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/nginx/certificates"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 7 })))
        .mount(&server)
        .await;

    let c = CertificatesClient::new(reqwest::Client::new(), server.uri(), "jwt".into());
    let id = c
        .request_letsencrypt(&LetsEncryptRequest {
            domain: "x.example.com".into(),
            letsencrypt_email: "a@b".into(),
            dns_provider_credentials: "dns_cloudflare_api_token = tk\n".into(),
        })
        .await
        .unwrap();
    assert_eq!(id, 7);
}

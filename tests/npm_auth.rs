use npm_docker_sync::npm::auth::{AuthClient, Credential};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn login_with_email_password_returns_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/tokens"))
        .and(body_json(json!({ "identity": "a@b", "secret": "pw" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "jwt-abc",
            "expires": "2099-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let client = AuthClient::new(reqwest::Client::new(), server.uri());
    let tok = client
        .login(&Credential::EmailPassword {
            email: "a@b".into(),
            password: "pw".into(),
        })
        .await
        .unwrap();
    assert_eq!(tok, "jwt-abc");
}

#[tokio::test]
async fn static_token_credential_returns_as_is() {
    let server = MockServer::start().await;
    let client = AuthClient::new(reqwest::Client::new(), server.uri());
    let tok = client
        .login(&Credential::Token("given".into()))
        .await
        .unwrap();
    assert_eq!(tok, "given");
}

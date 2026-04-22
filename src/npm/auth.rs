use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum Credential {
    EmailPassword { email: String, password: String },
    Token(String),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {0}")]
    Status(u16),
}

pub struct AuthClient {
    client: Client,
    base: String,
}

impl AuthClient {
    pub fn new(client: Client, base: String) -> Self {
        Self { client, base }
    }

    pub async fn login(&self, cred: &Credential) -> Result<String, AuthError> {
        match cred {
            Credential::Token(t) => Ok(t.clone()),
            Credential::EmailPassword { email, password } => {
                #[derive(Deserialize)]
                struct Resp {
                    token: String,
                }
                let res = self
                    .client
                    .post(format!("{}/api/tokens", self.base))
                    .json(&serde_json::json!({ "identity": email, "secret": password }))
                    .send()
                    .await?;
                if !res.status().is_success() {
                    return Err(AuthError::Status(res.status().as_u16()));
                }
                Ok(res.json::<Resp>().await?.token)
            }
        }
    }
}

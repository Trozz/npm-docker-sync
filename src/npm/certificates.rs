use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

pub struct LetsEncryptRequest {
    pub domain: String,
    pub letsencrypt_email: String,
    /// Full `dns_cloudflare_api_token = ...` INI snippet passed to certbot.
    pub dns_provider_credentials: String,
}

#[derive(Debug, Error)]
pub enum CertError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("status {0}")]
    Status(u16),
}

pub struct CertificatesClient {
    client: Client,
    base: String,
    token: String,
}

impl CertificatesClient {
    pub fn new(client: Client, base: String, token: String) -> Self {
        Self {
            client,
            base,
            token,
        }
    }

    pub async fn request_letsencrypt(&self, req: &LetsEncryptRequest) -> Result<u64, CertError> {
        #[derive(Deserialize)]
        struct Resp {
            id: u64,
        }
        let body = serde_json::json!({
            "provider": "letsencrypt",
            "domain_names": [req.domain],
            "meta": {
                "letsencrypt_email": req.letsencrypt_email,
                "letsencrypt_agree": true,
                "dns_challenge": true,
                "dns_provider": "cloudflare",
                "dns_provider_credentials": req.dns_provider_credentials,
                "propagation_seconds": 30
            }
        });
        let res = self
            .client
            .post(format!("{}/api/nginx/certificates", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(CertError::Status(res.status().as_u16()));
        }
        Ok(res.json::<Resp>().await?.id)
    }
}

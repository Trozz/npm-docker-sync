use crate::npm::types::*;
use reqwest::Client;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyHostError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("status {0}")]
    Status(u16),
}

pub struct ProxyHostClient {
    client: Client,
    base: String,
    token: String,
}

impl ProxyHostClient {
    pub fn new(client: Client, base: String, token: String) -> Self {
        Self {
            client,
            base,
            token,
        }
    }

    pub async fn list(&self) -> Result<Vec<ProxyHost>, ProxyHostError> {
        let res = self
            .client
            .get(format!("{}/api/nginx/proxy-hosts", self.base))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ProxyHostError::Status(res.status().as_u16()));
        }
        Ok(res.json().await?)
    }

    pub async fn create(&self, body: &CreateProxyHost) -> Result<ProxyHost, ProxyHostError> {
        let res = self
            .client
            .post(format!("{}/api/nginx/proxy-hosts", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ProxyHostError::Status(res.status().as_u16()));
        }
        Ok(res.json().await?)
    }

    pub async fn update(
        &self,
        id: u64,
        body: &UpdateProxyHost,
    ) -> Result<ProxyHost, ProxyHostError> {
        let res = self
            .client
            .put(format!("{}/api/nginx/proxy-hosts/{id}", self.base))
            .bearer_auth(&self.token)
            .json(&body.0)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ProxyHostError::Status(res.status().as_u16()));
        }
        Ok(res.json().await?)
    }

    pub async fn delete(&self, id: u64) -> Result<(), ProxyHostError> {
        let res = self
            .client
            .delete(format!("{}/api/nginx/proxy-hosts/{id}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ProxyHostError::Status(res.status().as_u16()));
        }
        Ok(())
    }
}

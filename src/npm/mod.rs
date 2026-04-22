pub mod auth;
pub mod certificates;
pub mod meta;
pub mod proxy_hosts;
pub mod types;

use std::sync::{Arc, RwLock};

use reqwest::Client;
use thiserror::Error;

use crate::npm::auth::{AuthClient, AuthError, Credential};
use crate::npm::certificates::{CertError, CertificatesClient, LetsEncryptRequest};
use crate::npm::proxy_hosts::{ProxyHostClient, ProxyHostError};
use crate::npm::types::*;

#[derive(Debug, Error)]
pub enum NpmError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    ProxyHost(#[from] ProxyHostError),
    #[error(transparent)]
    Cert(#[from] CertError),
}

/// Shared-token NPM client. Cloned freely across tasks via `clone_for_tasks`.
/// All clones observe JWT refreshes through the shared `Arc<RwLock<String>>`.
pub struct NpmClient {
    http: Client,
    base: String,
    cred: Credential,
    auth: AuthClient,
    token: Arc<RwLock<String>>,
}

impl NpmClient {
    pub async fn connect(base: String, cred: Credential) -> Result<Self, NpmError> {
        let http = Client::builder()
            .build()
            .expect("reqwest client builds with defaults");
        let auth = AuthClient::new(http.clone(), base.clone());
        let token = auth.login(&cred).await?;
        Ok(Self {
            http,
            base,
            cred,
            auth,
            token: Arc::new(RwLock::new(token)),
        })
    }

    /// Cheap clone for spawning into tasks. Shares the same token lock.
    pub fn clone_for_tasks(&self) -> Self {
        Self {
            http: self.http.clone(),
            base: self.base.clone(),
            cred: self.cred.clone(),
            auth: AuthClient::new(self.http.clone(), self.base.clone()),
            token: self.token.clone(),
        }
    }

    fn token_snapshot(&self) -> String {
        self.token.read().expect("token lock not poisoned").clone()
    }

    pub async fn list_proxy_hosts(&self) -> Result<Vec<ProxyHost>, NpmError> {
        let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token_snapshot());
        Ok(c.list().await?)
    }

    pub async fn create_proxy_host(&self, body: &CreateProxyHost) -> Result<ProxyHost, NpmError> {
        let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token_snapshot());
        Ok(c.create(body).await?)
    }

    pub async fn update_proxy_host(
        &self,
        id: u64,
        body: &UpdateProxyHost,
    ) -> Result<ProxyHost, NpmError> {
        let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token_snapshot());
        Ok(c.update(id, body).await?)
    }

    pub async fn delete_proxy_host(&self, id: u64) -> Result<(), NpmError> {
        let c = ProxyHostClient::new(self.http.clone(), self.base.clone(), self.token_snapshot());
        Ok(c.delete(id).await?)
    }

    pub async fn request_certificate(&self, req: &LetsEncryptRequest) -> Result<u64, NpmError> {
        let c =
            CertificatesClient::new(self.http.clone(), self.base.clone(), self.token_snapshot());
        Ok(c.request_letsencrypt(req).await?)
    }

    /// Called by the retry wrapper on 401 — refreshes the stored token visible to all clones.
    pub async fn refresh_token(&self) -> Result<(), NpmError> {
        let t = self.auth.login(&self.cred).await?;
        *self.token.write().expect("token lock not poisoned") = t;
        Ok(())
    }
}

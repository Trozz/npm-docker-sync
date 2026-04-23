pub mod labels;
pub mod spec;
pub mod watcher;

use std::collections::HashMap;

use bollard::Docker;
use bollard::models::{ContainerInspectResponse, ContainerSummary};
use bollard::query_parameters::ListContainersOptionsBuilder;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DockerError {
    #[error("docker: {0}")]
    Docker(#[from] bollard::errors::Error),
}

pub struct DockerClient {
    inner: Docker,
}

impl DockerClient {
    pub fn connect(socket: &str) -> Result<Self, DockerError> {
        let inner = if socket.starts_with("tcp://") || socket.starts_with("http://") {
            Docker::connect_with_http(socket, 120, bollard::API_DEFAULT_VERSION)?
        } else {
            Docker::connect_with_socket(socket, 120, bollard::API_DEFAULT_VERSION)?
        };
        Ok(Self { inner })
    }

    /// Shared reference to the underlying bollard client, for modules that need
    /// to call APIs we haven't wrapped (like the event stream in watcher.rs).
    pub fn handle(&self) -> &Docker {
        &self.inner
    }

    /// Clone for spawning into another task. `bollard::Docker` holds an `Arc` internally,
    /// so this is cheap.
    pub fn clone_for_tasks(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// List running containers that carry the `nginx_proxy_url` label.
    pub async fn list_labeled(&self) -> Result<Vec<ContainerSummary>, DockerError> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec!["nginx_proxy_url".to_string()]);

        let options = ListContainersOptionsBuilder::default()
            .filters(&filters)
            .build();

        let containers = self.inner.list_containers(Some(options)).await?;
        Ok(containers)
    }

    /// Inspect a single container by id.
    pub async fn inspect(&self, id: &str) -> Result<ContainerInspectResponse, DockerError> {
        let response = self.inner.inspect_container(id, None).await?;
        Ok(response)
    }
}

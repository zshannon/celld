//! URI-addressed secret providers, independent of the JavaScript runtime.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SecretBinding {
    #[serde(alias = "binding")]
    pub name: String,
    pub store_id: String,
    pub secret_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("Secret {0:?} not found")]
    NotFound(String),
    #[error("{0}")]
    Configuration(String),
    #[error("{0}")]
    Provider(String),
}

#[async_trait]
pub trait SecretProvider: Send + Sync {
    async fn get(&self, store_id: &str, secret_name: &str) -> Result<String, SecretError>;
}

#[derive(Default)]
pub struct Registry {
    providers: BTreeMap<String, Arc<dyn SecretProvider>>,
}

impl Registry {
    pub fn register(&mut self, scheme: impl Into<String>, provider: Arc<dyn SecretProvider>) {
        self.providers.insert(scheme.into(), provider);
    }

    pub async fn get(&self, store_id: &str, secret_name: &str) -> Result<String, SecretError> {
        let scheme = store_scheme(store_id)?;
        let provider = self.providers.get(scheme).ok_or_else(|| {
            SecretError::Configuration(format!("no secrets provider registered for {scheme:?}"))
        })?;
        provider.get(store_id, secret_name).await
    }
}

/// Provider URIs retain their original spelling for provider-specific names.
pub fn store_scheme(store_id: &str) -> Result<&str, SecretError> {
    store_id
        .split_once("://")
        .filter(|(scheme, store)| !scheme.is_empty() && !store.is_empty())
        .map(|(scheme, _)| scheme)
        .ok_or_else(|| SecretError::Configuration("secret store_id must be a provider URI".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait]
    impl SecretProvider for Echo {
        async fn get(&self, store_id: &str, secret_name: &str) -> Result<String, SecretError> {
            Ok(format!("{store_id}/{secret_name}"))
        }
    }

    #[tokio::test]
    async fn dispatches_the_configured_reference_to_its_provider() {
        let mut registry = Registry::default();
        registry.register("op", Arc::new(Echo));
        assert_eq!(
            registry
                .get("op://production", "stripe/api-key")
                .await
                .unwrap(),
            "op://production/stripe/api-key"
        );
    }

    #[tokio::test]
    async fn an_unregistered_scheme_is_a_configuration_error() {
        assert!(matches!(
            Registry::default()
                .get("op://production", "stripe/api-key")
                .await,
            Err(SecretError::Configuration(_))
        ));
    }
}

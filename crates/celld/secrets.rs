//! Host configuration and composition of secrets provider crates.
#![allow(clippy::disallowed_methods)]

use anyhow::{bail, Context};
use celld_secrets::Registry;
#[cfg(feature = "secrets-1password")]
use serde::Deserialize;
use std::collections::BTreeMap;
#[cfg(feature = "secrets-1password")]
use std::sync::Arc;

#[derive(Deserialize)]
#[cfg(feature = "secrets-1password")]
struct StoreConfig {
    endpoint: String,
    token_env: String,
}

pub fn from_env() -> anyhow::Result<Registry> {
    let config = std::env::var("CELLD_SECRET_STORES").unwrap_or_else(|_| "{}".into());
    from_config(&config, |name| std::env::var(name).ok())
}

fn from_config(config: &str, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Registry> {
    let stores: BTreeMap<String, serde_json::Value> = serde_json::from_str(config)
        .context("CELLD_SECRET_STORES must map store URIs to provider configurations")?;
    #[allow(unused_mut)]
    let mut registry = Registry::default();
    #[cfg(feature = "secrets-1password")]
    let mut connect = BTreeMap::new();
    for (uri, _config) in stores {
        match celld_secrets::store_scheme(&uri)? {
            #[cfg(feature = "secrets-1password")]
            "op" => {
                let config: StoreConfig = serde_json::from_value(_config).with_context(|| {
                    format!("invalid 1Password Connect configuration for {uri:?}")
                })?;
                let token = env(&config.token_env)
                    .filter(|token| !token.is_empty())
                    .with_context(|| {
                        format!(
                            "secret store {uri:?} requires host variable {}",
                            config.token_env
                        )
                    })?;
                connect.insert(
                    uri,
                    celld_secrets_1password::ConnectStore {
                        endpoint: config.endpoint,
                        token,
                    },
                );
            }
            scheme => bail!("secret provider {scheme:?} is not enabled in this celld build"),
        }
    }
    #[cfg(feature = "secrets-1password")]
    if !connect.is_empty() {
        registry.register(
            "op",
            Arc::new(celld_secrets_1password::ConnectProvider::new(connect)?),
        );
    }
    #[cfg(not(feature = "secrets-1password"))]
    let _ = env;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_config_needs_no_provider_credentials() {
        assert!(from_config("{}", |_| None).is_ok());
    }

    #[test]
    #[cfg(feature = "secrets-1password")]
    fn connect_config_reads_its_token_from_the_named_host_variable() {
        let config = r#"{"op://production":{"endpoint":"http://localhost:8080","token_env":"PRODUCTION_CONNECT_TOKEN"}}"#;
        assert!(from_config(config, |name| {
            assert_eq!(name, "PRODUCTION_CONNECT_TOKEN");
            Some("test-token".into())
        })
        .is_ok());
        assert!(from_config(config, |_| None)
            .err()
            .unwrap()
            .to_string()
            .contains("PRODUCTION_CONNECT_TOKEN"));
    }
}

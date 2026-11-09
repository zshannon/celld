//! 1Password Connect adapter using the connect-1password SDK.

use async_trait::async_trait;
use celld_secrets::{SecretError, SecretProvider};
use connect_1password::{client::Client, items, vaults};
use std::collections::BTreeMap;

pub struct ConnectStore {
    pub endpoint: String,
    pub token: String,
}

pub struct ConnectProvider {
    clients: BTreeMap<String, Client>,
}

impl ConnectProvider {
    pub fn new(stores: BTreeMap<String, ConnectStore>) -> Result<Self, SecretError> {
        Ok(Self {
            clients: stores
                .into_iter()
                .map(|(uri, store)| {
                    Ok((
                        uri,
                        Client::try_new(&store.token, store.endpoint.trim_end_matches('/'))
                            .map_err(provider_error)?,
                    ))
                })
                .collect::<Result<_, SecretError>>()?,
        })
    }
}

fn provider_error(error: connect_1password::error::Error) -> SecretError {
    SecretError::Provider(format!("1Password Connect: {error}"))
}

fn named(id: &str, name: &str, requested: &str) -> bool {
    id.eq_ignore_ascii_case(requested) || name.eq_ignore_ascii_case(requested)
}

#[async_trait]
impl SecretProvider for ConnectProvider {
    async fn get(&self, store_id: &str, secret_name: &str) -> Result<String, SecretError> {
        let client = self.clients.get(store_id).ok_or_else(|| {
            SecretError::Configuration(format!(
                "no 1Password Connect configuration for {store_id:?}"
            ))
        })?;
        let vault_name = store_id
            .strip_prefix("op://")
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .ok_or_else(|| {
                SecretError::Configuration("1Password store_id must be op://<vault>".into())
            })?;
        let parts: Vec<_> = secret_name.split('/').collect();
        let (item_name, section_name, field_name) = match parts.as_slice() {
            [item, field] if !item.is_empty() && !field.is_empty() => (*item, None, *field),
            [item, section, field]
                if !item.is_empty() && !section.is_empty() && !field.is_empty() =>
            {
                (*item, Some(*section), *field)
            }
            _ => {
                return Err(SecretError::Configuration(
                    "1Password secret_name must be <item>/[<section>/]<field>".into(),
                ))
            }
        };
        let missing = || SecretError::NotFound(secret_name.to_string());
        let (vaults, _) = vaults::all(client).await.map_err(provider_error)?;
        let vault = vaults
            .iter()
            .find(|vault| named(&vault.id, &vault.name, vault_name))
            .ok_or_else(missing)?;
        let (items, _) = items::all(client, &vault.id)
            .await
            .map_err(provider_error)?;
        let summary = items
            .iter()
            .find(|item| named(&item.id, &item.title, item_name))
            .ok_or_else(missing)?;
        let (item, raw) = items::get(client, &vault.id, &summary.id)
            .await
            .map_err(provider_error)?;
        let section_id = section_name
            .map(|name| {
                item.sections
                    .iter()
                    .find(|section| {
                        named(
                            &section.id,
                            section.label.as_deref().unwrap_or_default(),
                            name,
                        )
                    })
                    .map(|section| section.id.as_str())
                    .ok_or_else(missing)
            })
            .transpose()?;
        // The SDK's typed FieldObject omits field IDs; its raw result preserves
        // them so both names and IDs continue to work in secret references.
        raw["fields"]
            .as_array()
            .ok_or_else(missing)?
            .iter()
            .find(|field| {
                named(
                    field["id"].as_str().unwrap_or_default(),
                    field["label"].as_str().unwrap_or_default(),
                    field_name,
                ) && field["section"]["id"].as_str() == section_id
            })
            .map(|field| field["value"].as_str().unwrap_or_default().to_string())
            .ok_or_else(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn server(responses: Vec<(&'static str, &'static str)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            for (path, body) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 1024];
                    let len = socket.read(&mut bytes).unwrap();
                    assert!(len > 0);
                    request.extend_from_slice(&bytes[..len]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                assert!(
                    request.split_whitespace().nth(1).unwrap().split('?').next() == Some(path),
                    "{request}"
                );
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"));
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (endpoint, handle)
    }

    #[tokio::test]
    async fn resolves_a_vault_item_and_field_through_connect() {
        let (endpoint, handle) = server(vec![
            (
                "/v1/vaults",
                r#"[{"id":"vault-id","name":"production","attributeVersion":1,"contentVersion":1,"type":"USER_CREATED"}]"#,
            ),
            (
                "/v1/vaults/vault-id/items",
                r#"[{"id":"item-id","title":"stripe","vault":{"id":"vault-id"}}]"#,
            ),
            (
                "/v1/vaults/vault-id/items/item-id",
                r#"{"title":"stripe","vault":{"id":"vault-id"},"sections":[],"fields":[{"id":"field-id","label":"api-key","value":"test-secret"}]}"#,
            ),
        ]);
        let provider = ConnectProvider::new(BTreeMap::from([(
            "op://production".into(),
            ConnectStore {
                endpoint,
                token: "test-token".into(),
            },
        )]))
        .unwrap();
        assert_eq!(
            provider
                .get("op://production", "stripe/api-key")
                .await
                .unwrap(),
            "test-secret"
        );
        handle.join().unwrap();
    }

    #[test]
    fn sdk_client_debug_does_not_expose_its_token() {
        let client = Client::new(
            "test-credential-that-must-stay-private",
            "http://localhost:8080",
        );
        assert!(!format!("{client:?}").contains("test-credential-that-must-stay-private"));
    }

    #[test]
    fn sdk_accepts_connect_url_objects_and_optional_sections() {
        let summary = serde_json::json!({
            "id":"item-id", "title":"stripe", "vault":{"id":"vault-id"},
            "urls":[{"href":"https://example.com"}]
        });
        assert!(serde_json::from_value::<connect_1password::models::ItemData>(summary).is_ok());
        let item = serde_json::json!({
            "title":"stripe", "vault":{"id":"vault-id"},
            "fields":[{"id":"api-key", "type":"CONCEALED", "value":"test-secret"}]
        });
        assert!(serde_json::from_value::<connect_1password::models::FullItem>(item).is_ok());
    }

    #[tokio::test]
    async fn a_present_field_with_an_omitted_value_is_empty() {
        let (endpoint, handle) = server(vec![
            (
                "/v1/vaults",
                r#"[{"id":"vault-id","name":"production","attributeVersion":1,"contentVersion":1,"type":"USER_CREATED"}]"#,
            ),
            (
                "/v1/vaults/vault-id/items",
                r#"[{"id":"item-id","title":"stripe","vault":{"id":"vault-id"}}]"#,
            ),
            (
                "/v1/vaults/vault-id/items/item-id",
                r#"{"title":"stripe","vault":{"id":"vault-id"},"sections":[],"fields":[{"id":"notesPlain","type":"STRING"}]}"#,
            ),
        ]);
        let provider = ConnectProvider::new(BTreeMap::from([(
            "op://production".into(),
            ConnectStore {
                endpoint,
                token: "test-token".into(),
            },
        )]))
        .unwrap();
        assert_eq!(
            provider
                .get("op://production", "stripe/notesPlain")
                .await
                .unwrap(),
            ""
        );
        handle.join().unwrap();
    }
}

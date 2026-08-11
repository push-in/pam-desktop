use std::collections::HashMap;

use pam_desktop_protocol::{SecretOperation, validate_identifier};
use secret_service::{EncryptionType, SecretService};
use serde::{Deserialize, Serialize};

use crate::native::NativeError;

const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretRequest {
    pub operation: SecretOperation,
    pub window_id: String,
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretResponse {
    pub value: Option<String>,
}

pub async fn execute(
    application_id: &str,
    request: &SecretRequest,
) -> Result<SecretResponse, NativeError> {
    validate_identifier(&request.key, "secret key").map_err(NativeError::invalid)?;
    if request
        .value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_SECRET_BYTES)
    {
        return Err(NativeError::too_large("Secrets cannot exceed 64 KiB."));
    }
    let service = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(|error| NativeError::native("Cannot connect to Linux Secret Service", error))?;
    let attributes = HashMap::from([
        ("application", application_id),
        ("pam-key", request.key.as_str()),
    ]);

    match request.operation {
        SecretOperation::Read => {
            if request.value.is_some() {
                return Err(NativeError::invalid("Secret reads cannot include a value."));
            }
            let items = service.search_items(attributes).await.map_err(|error| {
                NativeError::native("Cannot search Linux Secret Service", error)
            })?;
            let item = if let Some(item) = items.unlocked.first() {
                Some(item)
            } else if let Some(item) = items.locked.first() {
                item.unlock()
                    .await
                    .map_err(|error| NativeError::native("Cannot unlock the secret", error))?;
                Some(item)
            } else {
                None
            };
            let value = if let Some(item) = item {
                let bytes = item
                    .get_secret()
                    .await
                    .map_err(|error| NativeError::native("Cannot read the secret", error))?;
                if bytes.len() > MAX_SECRET_BYTES {
                    return Err(NativeError::too_large("Stored secret exceeds 64 KiB."));
                }
                Some(
                    String::from_utf8(bytes)
                        .map_err(|_| NativeError::invalid("Stored secret is not valid UTF-8."))?,
                )
            } else {
                None
            };
            Ok(SecretResponse { value })
        }
        SecretOperation::Write => {
            let value = request
                .value
                .as_deref()
                .ok_or_else(|| NativeError::invalid("Secret writes require a value."))?;
            let collection = service.get_default_collection().await.map_err(|error| {
                NativeError::native("Cannot open the default Linux secret collection", error)
            })?;
            if collection.is_locked().await.map_err(|error| {
                NativeError::native("Cannot inspect the Linux secret collection", error)
            })? {
                collection.unlock().await.map_err(|error| {
                    NativeError::native("Cannot unlock the Linux secret collection", error)
                })?;
            }
            collection
                .create_item(
                    &format!("{application_id}: {}", request.key),
                    attributes,
                    value.as_bytes(),
                    true,
                    "text/plain; charset=utf-8",
                )
                .await
                .map_err(|error| NativeError::native("Cannot store the secret", error))?;
            Ok(SecretResponse { value: None })
        }
        SecretOperation::Delete => {
            if request.value.is_some() {
                return Err(NativeError::invalid(
                    "Secret deletion cannot include a value.",
                ));
            }
            let items = service.search_items(attributes).await.map_err(|error| {
                NativeError::native("Cannot search Linux Secret Service", error)
            })?;
            for item in items.unlocked.iter().chain(items.locked.iter()) {
                item.delete()
                    .await
                    .map_err(|error| NativeError::native("Cannot delete the secret", error))?;
            }
            Ok(SecretResponse { value: None })
        }
    }
}

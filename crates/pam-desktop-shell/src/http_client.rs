use std::collections::HashMap;
use std::time::Duration;

use pam_desktop_protocol::{ErrorCode, HttpMethod, HttpOriginConfig};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::native::NativeError;

const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpRequest {
    pub window_id: String,
    pub origin: String,
    #[serde(default)]
    pub method: HttpMethod,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: String,
}

pub struct HttpServices {
    origins: HashMap<String, Url>,
    agent: ureq::Agent,
}

impl HttpServices {
    pub fn prepare(configs: &[HttpOriginConfig]) -> Result<Self, String> {
        let mut origins = HashMap::with_capacity(configs.len());
        for config in configs {
            config.validate()?;
            origins.insert(
                config.name.clone(),
                Url::parse(&config.origin).map_err(|error| {
                    format!("cannot parse HTTP origin {:?}: {error}", config.name)
                })?,
            );
        }
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(HTTP_TIMEOUT))
            .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Ok(Self { origins, agent })
    }

    pub fn dispatch(&self, request: &HttpRequest) -> Result<Value, NativeError> {
        let origin = self
            .origins
            .get(&request.origin)
            .ok_or_else(|| NativeError::disabled(format!("HTTP origin {:?}", request.origin)))?;
        let url = resolve_url(origin, &request.path)?;
        validate_headers(&request.headers)?;
        if request.body.len() > MAX_REQUEST_BODY_BYTES {
            return Err(NativeError::too_large(format!(
                "Native HTTP request bodies are limited to {MAX_REQUEST_BODY_BYTES} bytes."
            )));
        }
        if matches!(
            request.method,
            HttpMethod::Get | HttpMethod::Delete | HttpMethod::Head
        ) && !request.body.is_empty()
        {
            return Err(NativeError::invalid(
                "GET, DELETE and HEAD requests cannot contain a body.",
            ));
        }

        let response = match request.method {
            HttpMethod::Get => apply_headers(self.agent.get(url.as_str()), &request.headers).call(),
            HttpMethod::Post => apply_headers(self.agent.post(url.as_str()), &request.headers)
                .send(request.body.as_bytes()),
            HttpMethod::Put => apply_headers(self.agent.put(url.as_str()), &request.headers)
                .send(request.body.as_bytes()),
            HttpMethod::Patch => apply_headers(self.agent.patch(url.as_str()), &request.headers)
                .send(request.body.as_bytes()),
            HttpMethod::Delete => {
                apply_headers(self.agent.delete(url.as_str()), &request.headers).call()
            }
            HttpMethod::Head => {
                apply_headers(self.agent.head(url.as_str()), &request.headers).call()
            }
        }
        .map_err(|error| NativeError {
            code: ErrorCode::NativeOperationFailed,
            message: format!("Native HTTP request failed: {error}"),
        })?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| !name.as_str().eq_ignore_ascii_case("set-cookie"))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), Value::String(value.to_owned())))
            })
            .collect::<Map<String, Value>>();
        let mut response = response;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BODY_BYTES)
            .read_to_vec()
            .map_err(|error| NativeError::too_large(format!(
                "Native HTTP response exceeds {MAX_RESPONSE_BODY_BYTES} bytes or cannot be read: {error}"
            )))?;
        let body = String::from_utf8(bytes).map_err(|_| {
            NativeError::invalid(
                "Native HTTP JSON responses must be UTF-8; stream binary downloads to a file.",
            )
        })?;
        Ok(json!({"status": status, "headers": headers, "body": body}))
    }
}

fn apply_headers<B>(
    mut request: ureq::RequestBuilder<B>,
    headers: &HashMap<String, String>,
) -> ureq::RequestBuilder<B> {
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request
}

fn resolve_url(origin: &Url, path: &str) -> Result<Url, NativeError> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
        return Err(NativeError::invalid(
            "Native HTTP paths must begin with one slash and cannot contain fragments.",
        ));
    }
    let base_path = origin.path().trim_end_matches('/');
    let url = Url::parse(&format!("{}{path}", origin.as_str().trim_end_matches('/'))).map_err(
        |error| NativeError::invalid(format!("The native HTTP path is invalid: {error}")),
    )?;
    let within_base = base_path.is_empty()
        || url.path() == base_path
        || url
            .path()
            .strip_prefix(base_path)
            .is_some_and(|suffix| suffix.starts_with('/'));
    if url.origin() != origin.origin() || !within_base {
        return Err(NativeError::permission(
            "The native HTTP request escapes its declared origin.",
        ));
    }
    Ok(url)
}

fn validate_headers(headers: &HashMap<String, String>) -> Result<(), NativeError> {
    if headers.len() > MAX_HEADERS {
        return Err(NativeError::too_large(format!(
            "Native HTTP requests accept at most {MAX_HEADERS} headers."
        )));
    }
    for (name, value) in headers {
        if name.is_empty()
            || name.len() > 128
            || value.len() > MAX_HEADER_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            || [
                "host",
                "connection",
                "content-length",
                "transfer-encoding",
                "cookie",
            ]
            .iter()
            .any(|blocked| name.eq_ignore_ascii_case(blocked))
            || value.contains(['\r', '\n'])
        {
            return Err(NativeError::invalid("The native HTTP headers are invalid."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confines_paths_to_the_declared_https_origin() {
        let origin = Url::parse("https://api.example.com/v1").expect("the origin should parse");
        assert_eq!(
            resolve_url(&origin, "/users?limit=1")
                .expect("the path should resolve")
                .as_str(),
            "https://api.example.com/v1/users?limit=1"
        );
        assert!(resolve_url(&origin, "//evil.example").is_err());
        assert!(resolve_url(&origin, "https://evil.example").is_err());
    }

    #[test]
    fn rejects_hop_by_hop_and_cookie_headers() {
        assert!(
            validate_headers(&HashMap::from([(
                "Cookie".to_owned(),
                "session=secret".to_owned(),
            )]))
            .is_err()
        );
    }
}

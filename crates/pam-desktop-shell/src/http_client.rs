use std::collections::HashMap;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use pam_desktop_protocol::{ErrorCode, HttpMethod, HttpOriginConfig};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::desktop_otlp::parse_traceparent;
use crate::native::NativeError;

const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(try_from = "u8")]
#[repr(u8)]
pub enum HttpBodyEncoding {
    #[default]
    Utf8 = 1,
    Base64 = 2,
}

impl TryFrom<u8> for HttpBodyEncoding {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Utf8),
            2 => Ok(Self::Base64),
            _ => Err("HTTP body encoding must be 1 or 2.".to_owned()),
        }
    }
}

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
    #[serde(default)]
    pub body_encoding: HttpBodyEncoding,
    #[serde(default)]
    pub response_body_encoding: HttpBodyEncoding,
    pub traceparent: Option<String>,
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
        let traceparent = validated_traceparent(request.traceparent.as_deref())?;
        let body = decode_body(request)?;
        if body.len() > MAX_REQUEST_BODY_BYTES {
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
            HttpMethod::Get => apply_headers(
                self.agent.get(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .call(),
            HttpMethod::Post => apply_headers(
                self.agent.post(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .send(&body),
            HttpMethod::Put => apply_headers(
                self.agent.put(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .send(&body),
            HttpMethod::Patch => apply_headers(
                self.agent.patch(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .send(&body),
            HttpMethod::Delete => apply_headers(
                self.agent.delete(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .call(),
            HttpMethod::Head => apply_headers(
                self.agent.head(url.as_str()),
                &request.headers,
                traceparent.as_deref(),
            )
            .call(),
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
        let body = encode_response_body(bytes, request.response_body_encoding)?;
        Ok(json!({
            "status": status,
            "headers": headers,
            "body": body,
            "bodyEncoding": request.response_body_encoding as u8,
        }))
    }
}

fn encode_response_body(bytes: Vec<u8>, encoding: HttpBodyEncoding) -> Result<String, NativeError> {
    match encoding {
        HttpBodyEncoding::Utf8 => String::from_utf8(bytes).map_err(|_| {
            NativeError::invalid(
                "Native HTTP text responses must be UTF-8; request Base64 encoding for binary data.",
            )
        }),
        HttpBodyEncoding::Base64 => Ok(BASE64.encode(bytes)),
    }
}

fn decode_body(request: &HttpRequest) -> Result<Vec<u8>, NativeError> {
    match request.body_encoding {
        HttpBodyEncoding::Utf8 => Ok(request.body.as_bytes().to_vec()),
        HttpBodyEncoding::Base64 => BASE64
            .decode(request.body.as_bytes())
            .map_err(|_| NativeError::invalid("Base64 HTTP request bodies are invalid.")),
    }
}

fn apply_headers<B>(
    mut request: ureq::RequestBuilder<B>,
    headers: &HashMap<String, String>,
    traceparent: Option<&str>,
) -> ureq::RequestBuilder<B> {
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(traceparent) = traceparent {
        request = request.header("traceparent", traceparent);
    }
    request
}

fn validated_traceparent(value: Option<&str>) -> Result<Option<String>, NativeError> {
    value
        .map(|value| {
            parse_traceparent(value).map_or_else(
                || {
                    Err(NativeError::invalid(
                        "The native HTTP traceparent is invalid.",
                    ))
                },
                |parent| Ok(parent.header_value()),
            )
        })
        .transpose()
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
                "traceparent",
                "tracestate",
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
    use std::io::{Read, Write};
    use std::net::TcpListener;

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
        assert!(
            validate_headers(&HashMap::from([(
                "traceparent".to_owned(),
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_owned(),
            )]))
            .is_err()
        );
        assert!(
            validate_headers(&HashMap::from([(
                "TrAcEsTaTe".to_owned(),
                "vendor=value".to_owned(),
            )]))
            .is_err()
        );
    }

    #[test]
    fn decodes_binary_http_request_bodies() {
        let request = HttpRequest {
            window_id: "main".to_owned(),
            origin: "uploads".to_owned(),
            method: HttpMethod::Put,
            path: "/object".to_owned(),
            headers: HashMap::new(),
            body: "AP+A".to_owned(),
            body_encoding: HttpBodyEncoding::Base64,
            response_body_encoding: HttpBodyEncoding::Utf8,
            traceparent: None,
        };
        assert_eq!(decode_body(&request).unwrap(), vec![0, 255, 128]);
    }

    #[test]
    fn encodes_binary_http_response_bodies() {
        assert_eq!(
            encode_response_body(vec![0, 255, 128], HttpBodyEncoding::Base64).unwrap(),
            "AP+A"
        );
        assert!(encode_response_body(vec![255], HttpBodyEncoding::Utf8).is_err());
    }

    #[test]
    fn validates_dedicated_outbound_trace_context() {
        let valid = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        assert_eq!(
            validated_traceparent(Some(valid)).unwrap().as_deref(),
            Some(valid)
        );
        assert!(validated_traceparent(Some("forged")).is_err());
        assert!(validated_traceparent(None).unwrap().is_none());
    }

    #[test]
    fn host_injects_validated_traceparent_after_application_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4_096];
            let read = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&request[..read]).into_owned()
        });
        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .build()
            .new_agent();
        apply_headers(
            agent.get(format!("http://{address}/resource")),
            &HashMap::from([("accept".to_owned(), "application/json".to_owned())]),
            Some(traceparent),
        )
        .call()
        .unwrap();
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.contains(&format!("traceparent: {traceparent}")));
        assert!(request.contains("accept: application/json"));
    }
}

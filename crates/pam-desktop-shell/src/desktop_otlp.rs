use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pam_desktop_protocol::CommandExecution;
use serde_json::{Value, json};
use url::Url;

const DEFAULT_QUEUE_SIZE: usize = 2_048;
const DEFAULT_BATCH_SIZE: usize = 512;
const DEFAULT_DELAY_MS: u64 = 5_000;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_RESPONSE_BYTES: u64 = 65_536;

#[derive(Clone, Default)]
pub(crate) struct OtlpCounters {
    pub exported: Arc<AtomicU64>,
    pub dropped: Arc<AtomicU64>,
    pub errors: Arc<AtomicU64>,
    pub rejected: Arc<AtomicU64>,
}

#[derive(Clone)]
pub(crate) struct DesktopOtlpExporter {
    sender: SyncSender<CommandSpan>,
    pub counters: OtlpCounters,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum CommandOutcome {
    Success = 1,
    HandlerFailure = 2,
    WorkerFailure = 3,
    TaskFailure = 4,
}

pub(crate) struct CommandSpan {
    command: String,
    execution: CommandExecution,
    outcome: CommandOutcome,
    start_unix_nano: u64,
    end_unix_nano: u64,
    parent: Option<TraceParent>,
}

#[derive(Clone)]
pub(crate) struct TraceParent {
    trace_id: String,
    parent_span_id: String,
    flags: u8,
}

struct Config {
    endpoint: Url,
    headers: Vec<(String, String)>,
    service_name: String,
    batch_size: usize,
    delay: Duration,
    timeout: Duration,
}

impl DesktopOtlpExporter {
    pub(crate) fn from_environment() -> Result<Option<Self>, String> {
        if !enabled()? {
            return Ok(None);
        }
        let config = Config::from_environment()?;
        let queue_size = env_usize("OTEL_BSP_MAX_QUEUE_SIZE", DEFAULT_QUEUE_SIZE, 1, 65_536)?;
        let counters = OtlpCounters::default();
        let (sender, receiver) = sync_channel(queue_size);
        let worker_counters = counters.clone();
        std::thread::Builder::new()
            .name("pam-desktop-otlp".to_owned())
            .spawn(move || export_loop(&config, &receiver, &worker_counters))
            .map_err(|error| format!("cannot start Desktop OTLP exporter: {error}"))?;
        Ok(Some(Self { sender, counters }))
    }

    pub(crate) fn export(
        &self,
        command: String,
        execution: CommandExecution,
        outcome: CommandOutcome,
        start_unix_nano: u64,
        parent: Option<TraceParent>,
    ) {
        let span = CommandSpan {
            command,
            execution,
            outcome,
            start_unix_nano,
            end_unix_nano: epoch_nanos(),
            parent,
        };
        if let Err(error) = self.sender.try_send(span) {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            if matches!(error, TrySendError::Disconnected(_)) {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Config {
    fn from_environment() -> Result<Self, String> {
        let signal_endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").ok();
        let global_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
        let raw_endpoint = signal_endpoint
            .as_ref()
            .or(global_endpoint.as_ref())
            .ok_or_else(|| {
                "PAM_DESKTOP_OTLP_ENABLED requires OTEL_EXPORTER_OTLP_TRACES_ENDPOINT or OTEL_EXPORTER_OTLP_ENDPOINT"
                    .to_owned()
            })?;
        let protocol = std::env::var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL")
            .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL"))
            .unwrap_or_else(|_| "http/protobuf".to_owned());
        if protocol != "http/json" {
            return Err(format!(
                "Desktop OTLP requires OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/json (got {protocol:?})"
            ));
        }
        let mut endpoint = Url::parse(raw_endpoint)
            .map_err(|_| "Desktop OTLP endpoint is not a valid URL".to_owned())?;
        if signal_endpoint.is_none() {
            endpoint.set_path(&format!(
                "{}/v1/traces",
                endpoint.path().trim_end_matches('/')
            ));
        }
        validate_endpoint(&endpoint)?;
        let headers = parse_headers(
            &std::env::var("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
                .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_HEADERS"))
                .unwrap_or_default(),
        )?;
        let queue_size = env_usize("OTEL_BSP_MAX_QUEUE_SIZE", DEFAULT_QUEUE_SIZE, 1, 65_536)?;
        let batch_size = env_usize(
            "OTEL_BSP_MAX_EXPORT_BATCH_SIZE",
            DEFAULT_BATCH_SIZE.min(queue_size),
            1,
            queue_size,
        )?;
        let delay_ms = env_u64("OTEL_BSP_SCHEDULE_DELAY", DEFAULT_DELAY_MS, 1, 60_000)?;
        let timeout_ms = env_u64_fallback(
            "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
            "OTEL_EXPORTER_OTLP_TIMEOUT",
            DEFAULT_TIMEOUT_MS,
            1,
            120_000,
        )?;
        let service_name =
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "pam-desktop".to_owned());
        if service_name.is_empty()
            || service_name.len() > 128
            || service_name.chars().any(char::is_control)
        {
            return Err("OTEL_SERVICE_NAME must contain 1..128 printable characters".to_owned());
        }
        Ok(Self {
            endpoint,
            headers,
            service_name,
            batch_size,
            delay: Duration::from_millis(delay_ms),
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

fn enabled() -> Result<bool, String> {
    match std::env::var("PAM_DESKTOP_OTLP_ENABLED") {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Ok(value) if matches!(value.as_str(), "1" | "true") => Ok(true),
        Ok(value) if matches!(value.as_str(), "0" | "false") => Ok(false),
        Ok(_) => Err("PAM_DESKTOP_OTLP_ENABLED must be 1, true, 0, or false".to_owned()),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("PAM_DESKTOP_OTLP_ENABLED must be valid UTF-8".to_owned())
        }
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<(), String> {
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err("Desktop OTLP endpoint must not contain credentials or a fragment".to_owned());
    }
    let loopback = endpoint.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
        return Err(
            "Desktop OTLP endpoint must use HTTPS; HTTP is allowed only on loopback".to_owned(),
        );
    }
    Ok(())
}

fn parse_headers(raw: &str) -> Result<Vec<(String, String)>, String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (name, value) = entry
                .split_once('=')
                .ok_or_else(|| "Desktop OTLP headers must use key=value entries".to_owned())?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err("Desktop OTLP header name is invalid".to_owned());
            }
            let value = percent_decode(value)?;
            if value.contains(['\r', '\n']) {
                return Err("Desktop OTLP header value is invalid".to_owned());
            }
            Ok((name.to_owned(), value))
        })
        .collect()
}

fn percent_decode(value: &str) -> Result<String, String> {
    let mut output = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = bytes
                .get(index + 1..index + 3)
                .ok_or_else(|| "Desktop OTLP header has invalid percent encoding".to_owned())?;
            let encoded = std::str::from_utf8(encoded)
                .map_err(|_| "Desktop OTLP header has invalid percent encoding".to_owned())?;
            output.push(
                u8::from_str_radix(encoded, 16)
                    .map_err(|_| "Desktop OTLP header has invalid percent encoding".to_owned())?,
            );
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| "Desktop OTLP header is not valid UTF-8".to_owned())
}

fn export_loop(config: &Config, receiver: &Receiver<CommandSpan>, counters: &OtlpCounters) {
    let agent = ureq::Agent::config_builder()
        .https_only(config.endpoint.scheme() == "https")
        .max_redirects(0)
        .timeout_global(Some(config.timeout))
        .user_agent(concat!("pam-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    while let Ok(first) = receiver.recv() {
        let mut batch = Vec::with_capacity(config.batch_size);
        batch.push(first);
        let deadline = Instant::now() + config.delay;
        while batch.len() < config.batch_size {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(span) => batch.push(span),
                Err(_) => break,
            }
        }
        let count = batch.len() as u64;
        let body = payload(&config.service_name, &batch);
        if let Ok(rejected) = send(&agent, config, &body) {
            counters
                .exported
                .fetch_add(count.saturating_sub(rejected), Ordering::Relaxed);
            counters.rejected.fetch_add(rejected, Ordering::Relaxed);
        } else {
            counters.errors.fetch_add(1, Ordering::Relaxed);
            counters.dropped.fetch_add(count, Ordering::Relaxed);
        }
    }
}

fn send(agent: &ureq::Agent, config: &Config, body: &Value) -> Result<u64, ()> {
    let mut request = agent
        .post(config.endpoint.as_str())
        .header("content-type", "application/json");
    for (name, value) in &config.headers {
        request = request.header(name, value);
    }
    let mut response = request.send_json(body).map_err(|_| ())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|_| ())?;
    if bytes.is_empty() {
        return Ok(0);
    }
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let rejected = response
        .pointer("/partialSuccess/rejectedSpans")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .unwrap_or(0);
    Ok(rejected)
}

fn payload(service_name: &str, spans: &[CommandSpan]) -> Value {
    let spans = spans
        .iter()
        .map(|span| {
            let error = !matches!(span.outcome, CommandOutcome::Success);
            let trace_id = span
                .parent
                .as_ref()
                .map_or_else(random_hex::<16>, |parent| parent.trace_id.clone());
            let parent_span_id = span
                .parent
                .as_ref()
                .map(|parent| parent.parent_span_id.clone());
            let flags = span.parent.as_ref().map_or(1, |parent| parent.flags);
            let mut encoded = json!({
                "traceId": trace_id,
                "spanId": random_hex::<8>(),
                "name": "pam.desktop.command",
                "kind": 1,
                "startTimeUnixNano": span.start_unix_nano.to_string(),
                "endTimeUnixNano": span.end_unix_nano.to_string(),
                "attributes": [
                    {"key": "rpc.system", "value": {"stringValue": "pam.desktop"}},
                    {"key": "rpc.method", "value": {"stringValue": span.command}},
                    {"key": "pam.desktop.execution", "value": {"intValue": (span.execution as u8).to_string()}},
                    {"key": "pam.desktop.outcome", "value": {"intValue": (span.outcome as u8).to_string()}},
                ],
                "status": {"code": if error { 2 } else { 1 }},
                "flags": flags,
            });
            if let Some(parent_span_id) = parent_span_id {
                encoded["parentSpanId"] = Value::String(parent_span_id);
            }
            encoded
        })
        .collect::<Vec<_>>();
    json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": service_name}},
                {"key": "service.version", "value": {"stringValue": env!("CARGO_PKG_VERSION")}},
            ]},
            "scopeSpans": [{
                "scope": {"name": "pam.desktop", "version": env!("CARGO_PKG_VERSION")},
                "spans": spans,
            }],
        }],
    })
}

pub(crate) fn parse_traceparent(value: &str) -> Option<TraceParent> {
    let parts = value.split('-').collect::<Vec<_>>();
    let valid = parts.len() == 4
        && parts[0] == "00"
        && parts[1].len() == 32
        && parts[1] != "00000000000000000000000000000000"
        && parts[2].len() == 16
        && parts[2] != "0000000000000000"
        && parts[3].len() == 2
        && parts.iter().all(|part| {
            part.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    valid.then(|| TraceParent {
        trace_id: parts[1].to_owned(),
        parent_span_id: parts[2].to_owned(),
        flags: u8::from_str_radix(parts[3], 16).expect("validated trace flags"),
    })
}

fn random_hex<const N: usize>() -> String {
    let mut bytes = [0_u8; N];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = epoch_nanos().to_le_bytes();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = fallback[index % fallback.len()] ^ u8::try_from(index).unwrap_or(u8::MAX);
        }
    }
    bytes
        .iter()
        .fold(String::with_capacity(N * 2), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

pub(crate) fn epoch_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        })
}

fn env_usize(name: &str, default: usize, minimum: usize, maximum: usize) -> Result<usize, String> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(default);
    };
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}

fn env_u64(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64, String> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(default);
    };
    let value = raw
        .parse::<u64>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}

fn env_u64_fallback(
    primary: &str,
    fallback: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, String> {
    match std::env::var(primary).or_else(|_| std::env::var(fallback)) {
        Ok(raw) => {
            let value = raw
                .parse::<u64>()
                .map_err(|_| format!("{primary} must be an integer"))?;
            if !(minimum..=maximum).contains(&value) {
                return Err(format!("{primary} must be between {minimum} and {maximum}"));
            }
            Ok(value)
        }
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn payload_contains_only_operational_command_fields() {
        let value = payload(
            "desktop-test",
            &[CommandSpan {
                command: "catalog.refresh".to_owned(),
                execution: CommandExecution::Parallel,
                outcome: CommandOutcome::Success,
                start_unix_nano: 1,
                end_unix_nano: 2,
                parent: None,
            }],
        );
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(encoded.contains("catalog.refresh"));
        assert!(!encoded.contains("windowId"));
        assert!(!encoded.contains("requestId"));
        assert!(!encoded.contains("payload"));
        assert!(!encoded.contains("bridge"));
    }

    #[test]
    fn remote_http_and_header_injection_are_rejected() {
        assert!(
            validate_endpoint(&Url::parse("http://collector.example/v1/traces").unwrap()).is_err()
        );
        assert!(validate_endpoint(&Url::parse("http://127.0.0.1:4318/v1/traces").unwrap()).is_ok());
        assert!(parse_headers("authorization=Bearer%20safe").is_ok());
        assert!(parse_headers("authorization=safe%0d%0aleak").is_err());
    }

    #[test]
    fn coded_outcomes_are_sequential() {
        assert_eq!(CommandOutcome::Success as u8, 1);
        assert_eq!(CommandOutcome::HandlerFailure as u8, 2);
        assert_eq!(CommandOutcome::WorkerFailure as u8, 3);
        assert_eq!(CommandOutcome::TaskFailure as u8, 4);
    }

    #[test]
    fn traceparent_requires_lowercase_nonzero_w3c_version_zero_ids() {
        let valid = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parent = parse_traceparent(valid).expect("valid W3C traceparent");
        assert_eq!(parent.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(parent.parent_span_id, "00f067aa0ba902b7");
        assert_eq!(parent.flags, 1);
        assert!(parse_traceparent(&valid.to_uppercase()).is_none());
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01").is_none()
        );
        assert!(
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01").is_none()
        );
    }

    #[test]
    fn payload_preserves_authenticated_parent_lineage() {
        let parent =
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00").unwrap();
        let value = payload(
            "desktop-test",
            &[CommandSpan {
                command: "catalog.refresh".to_owned(),
                execution: CommandExecution::Stateful,
                outcome: CommandOutcome::Success,
                start_unix_nano: 1,
                end_unix_nano: 2,
                parent: Some(parent),
            }],
        );
        let span = &value["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["traceId"], "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(span["parentSpanId"], "00f067aa0ba902b7");
        assert_eq!(span["flags"], 0);
    }

    #[test]
    fn partial_success_accepts_otlp_json_uint64_strings() {
        let response = json!({"partialSuccess": {"rejectedSpans": "3"}});
        let rejected = response
            .pointer("/partialSuccess/rejectedSpans")
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
            });
        assert_eq!(rejected, Some(3));
    }

    #[test]
    #[ignore = "requires the official OTLP Collector started by scripts/certify-desktop-otlp.sh"]
    fn official_collector_accepts_desktop_command_span() {
        let exporter = DesktopOtlpExporter::from_environment()
            .expect("valid certification environment")
            .expect("Desktop OTLP must be enabled");
        exporter.export(
            "catalog.refresh".to_owned(),
            CommandExecution::Parallel,
            CommandOutcome::Success,
            epoch_nanos(),
            Some(
                parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
                    .unwrap(),
            ),
        );
        for _ in 0..100 {
            if exporter.counters.exported.load(Ordering::Relaxed) == 1 {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("Desktop command span was not accepted by the Collector");
    }
}

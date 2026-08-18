use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pam_desktop_protocol::{
    Bootstrap, FileAccess, NativeCapabilities, ProcessArgumentPolicy, RustPluginConfig,
};
use serde::Serialize;

use crate::project::Project;
use crate::runtime::DesktopRuntime;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
enum Severity {
    Info = 1,
    Warning = 2,
    High = 3,
    Critical = 4,
}

impl Severity {
    const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finding {
    severity_code: u8,
    rule: &'static str,
    resource: String,
    message: &'static str,
    remediation: &'static str,
}

impl Finding {
    fn new(
        severity: Severity,
        rule: &'static str,
        resource: impl Into<String>,
        message: &'static str,
        remediation: &'static str,
    ) -> Self {
        Self {
            severity_code: severity as u8,
            rule,
            resource: resource.into(),
            message,
            remediation,
        }
    }

    fn severity(&self) -> Severity {
        match self.severity_code {
            1 => Severity::Info,
            2 => Severity::Warning,
            3 => Severity::High,
            4 => Severity::Critical,
            _ => unreachable!("findings are created from Severity"),
        }
    }
}

#[derive(Debug)]
struct Options {
    project: PathBuf,
    json: bool,
    deny: Severity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    info: usize,
    warning: usize,
    high: usize,
    critical: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    schema_version: u8,
    surface_code: u8,
    result_code: u8,
    deny_severity_code: u8,
    application_identifier: &'a str,
    counts: Counts,
    findings: &'a [Finding],
}

pub fn run(arguments: Vec<OsString>) -> Result<(), String> {
    let options = parse(arguments)?;
    let runtime = DesktopRuntime::prepare(Project::discover(&options.project)?)?;
    let findings = audit(runtime.project().root(), runtime.bootstrap());
    let failed = findings
        .iter()
        .any(|finding| finding.severity() >= options.deny);
    let report = Report {
        schema_version: 1,
        surface_code: 3,
        result_code: if failed { 2 } else { 1 },
        deny_severity_code: options.deny as u8,
        application_identifier: &runtime.bootstrap().manifest.identifier,
        counts: counts(&findings),
        findings: &findings,
    };

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("cannot encode permission audit: {error}"))?
        );
    } else {
        print_human(&report);
    }
    if failed {
        return Err(format!(
            "permission audit failed at severity {} or higher",
            options.deny.label()
        ));
    }
    Ok(())
}

fn parse(arguments: Vec<OsString>) -> Result<Options, String> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("permissions")) {
        return Err(usage());
    }
    let mut project = PathBuf::from(".");
    let mut positional = false;
    let mut json = false;
    let mut deny = Severity::Critical;
    for argument in arguments {
        match argument.to_str() {
            Some("--json") => json = true,
            Some("--deny-high") => deny = Severity::High,
            Some(value) if !value.starts_with('-') && !positional => {
                project = PathBuf::from(value);
                positional = true;
            }
            _ => return Err(usage()),
        }
    }
    Ok(Options {
        project,
        json,
        deny,
    })
}

fn usage() -> String {
    "usage: pam-desktop audit permissions [directory] [--json] [--deny-high]".to_owned()
}

fn audit(project_root: &Path, bootstrap: &Bootstrap) -> Vec<Finding> {
    let mut findings = Vec::new();
    audit_capabilities(project_root, &bootstrap.capabilities, &mut findings);
    audit_plugins(&bootstrap.rust_plugins, &mut findings);

    if bootstrap.manifest.lifecycle.autostart {
        findings.push(Finding::new(
            Severity::High,
            "lifecycle.autostart",
            "application",
            "The application starts automatically with the user session.",
            "Keep autostart disabled unless continuous background execution is essential and disclosed.",
        ));
    }
    if bootstrap.manifest.updates.is_none() {
        findings.push(Finding::new(
            Severity::Info,
            "updates.disabled",
            "application",
            "Signed automatic update delivery is not configured.",
            "Document the external patch channel or configure a signed HTTPS update feed before release.",
        ));
    }
    if !bootstrap.background_jobs.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "runtime.background-jobs",
            format!("{} jobs", bootstrap.background_jobs.len()),
            "PHP jobs execute while the desktop host is running.",
            "Review every schedule, overlap policy and command for least privilege and bounded work.",
        ));
    }
    if !bootstrap.shell.shortcuts.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "shell.global-shortcuts",
            format!("{} shortcuts", bootstrap.shell.shortcuts.len()),
            "Global shortcuts receive input outside the application windows.",
            "Register only essential shortcuts and disclose them in application settings.",
        ));
    }

    findings.sort_by(|left, right| {
        right
            .severity_code
            .cmp(&left.severity_code)
            .then_with(|| left.rule.cmp(right.rule))
            .then_with(|| left.resource.cmp(&right.resource))
    });
    findings
}

fn audit_capabilities(
    project_root: &Path,
    capabilities: &NativeCapabilities,
    findings: &mut Vec<Finding>,
) {
    audit_filesystem(project_root, capabilities, findings);
    audit_sensitive_capabilities(capabilities, findings);
    audit_network_and_processes(capabilities, findings);
}

fn audit_filesystem(
    project_root: &Path,
    capabilities: &NativeCapabilities,
    findings: &mut Vec<Finding>,
) {
    for root in &capabilities.filesystem_roots {
        let configured = PathBuf::from(&root.path);
        let candidate = if configured.is_absolute() {
            configured
        } else {
            project_root.join(configured)
        };
        let Ok(resolved) = candidate.canonicalize() else {
            findings.push(Finding::new(
                Severity::Critical,
                "filesystem.unresolved",
                &root.name,
                "The filesystem capability cannot be resolved to a stable directory.",
                "Create the directory before auditing and reject symlinks or missing release paths.",
            ));
            continue;
        };
        let external = !resolved.starts_with(project_root);
        let broad = project_root.starts_with(&resolved);
        let writable = matches!(root.access, FileAccess::Write | FileAccess::ReadWrite);
        let (severity, rule, message, remediation) = if writable && (external || broad) {
            (
                Severity::Critical,
                "filesystem.broad-write",
                "A writable filesystem root includes the whole project or an external directory.",
                "Restrict writes to a dedicated project data directory with the narrowest access mode.",
            )
        } else if external || broad {
            (
                Severity::High,
                "filesystem.broad-read",
                "A readable filesystem root includes the whole project or an external directory.",
                "Copy required assets into a dedicated read-only project directory.",
            )
        } else if writable {
            (
                Severity::Warning,
                "filesystem.write",
                "The renderer can request writes inside this named root.",
                "Keep generated and user data isolated from code, credentials and bundled assets.",
            )
        } else {
            continue;
        };
        findings.push(Finding::new(
            severity,
            rule,
            &root.name,
            message,
            remediation,
        ));
    }
}

fn audit_sensitive_capabilities(capabilities: &NativeCapabilities, findings: &mut Vec<Finding>) {
    if capabilities.clipboard_read {
        findings.push(Finding::new(
            Severity::High,
            "clipboard.read",
            "clipboard",
            "The renderer can read clipboard contents, which may contain secrets.",
            "Prefer paste events or an explicit user action and disable ambient clipboard reads.",
        ));
    }
    if capabilities.clipboard_write {
        findings.push(Finding::new(
            Severity::Warning,
            "clipboard.write",
            "clipboard",
            "The renderer can replace the user's clipboard contents.",
            "Expose clipboard writes only from a visible user gesture.",
        ));
    }
    if capabilities.secrets {
        findings.push(Finding::new(
            Severity::High,
            "secrets.access",
            "secret-service",
            "Application commands can access operating-system secret storage.",
            "Keep secret operations behind narrow PHP commands and never return secret values to untrusted views.",
        ));
    }
    if capabilities.dialogs || capabilities.drag_and_drop || capabilities.desktop_portal {
        findings.push(Finding::new(
            Severity::Warning,
            "user-mediated.external-access",
            "desktop-integration",
            "User-mediated integrations can grant temporary access to external files or desktop services.",
            "Preserve explicit consent and avoid persisting grants or ambient paths.",
        ));
    }
    if capabilities.system_information {
        findings.push(Finding::new(
            Severity::Warning,
            "system.information",
            "system-information",
            "The application can inspect bounded host, network, power and memory metadata.",
            "Collect only operational fields and disclose diagnostics collection to users.",
        ));
    }
}

fn audit_network_and_processes(capabilities: &NativeCapabilities, findings: &mut Vec<Finding>) {
    for origin in &capabilities.http_origins {
        if origin_base_path(&origin.origin) == "/" {
            findings.push(Finding::new(
                Severity::Warning,
                "http.origin-wide",
                &origin.name,
                "The native HTTP capability allows every path on its HTTPS origin.",
                "Declare the narrowest stable API base path instead of the origin root.",
            ));
        }
    }
    for process in &capabilities.processes {
        findings.push(Finding::new(
            if process.argument_policy == ProcessArgumentPolicy::Append {
                Severity::High
            } else {
                Severity::Warning
            },
            if process.argument_policy == ProcessArgumentPolicy::Append {
                "process.dynamic-arguments"
            } else {
                "process.execution"
            },
            &process.name,
            if process.argument_policy == ProcessArgumentPolicy::Append {
                "The renderer can append runtime arguments to a bundled executable."
            } else {
                "The renderer can launch a bundled executable with fixed arguments."
            },
            "Prefer fixed arguments and validate all process inputs in a narrow PHP command.",
        ));
    }
}

fn audit_plugins(plugins: &[RustPluginConfig], findings: &mut Vec<Finding>) {
    for plugin in plugins {
        findings.push(Finding::new(
            if plugin.sha256.is_some() {
                Severity::Warning
            } else {
                Severity::Critical
            },
            if plugin.sha256.is_some() {
                "plugin.process"
            } else {
                "plugin.unpinned"
            },
            &plugin.id,
            if plugin.sha256.is_some() {
                "A process-isolated Rust plugin receives application-defined native commands."
            } else {
                "A Rust plugin executable is not pinned to a SHA-256 digest."
            },
            if plugin.sha256.is_some() {
                "Keep the command allowlist minimal and review plugin updates before changing the digest."
            } else {
                "Set the plugin SHA-256 digest and update it only through a reviewed release process."
            },
        ));
    }
}

fn origin_base_path(origin: &str) -> &str {
    let after_scheme = origin.strip_prefix("https://").unwrap_or(origin);
    after_scheme
        .find('/')
        .map_or("/", |index| &after_scheme[index..])
}

fn counts(findings: &[Finding]) -> Counts {
    Counts {
        info: findings
            .iter()
            .filter(|finding| finding.severity() == Severity::Info)
            .count(),
        warning: findings
            .iter()
            .filter(|finding| finding.severity() == Severity::Warning)
            .count(),
        high: findings
            .iter()
            .filter(|finding| finding.severity() == Severity::High)
            .count(),
        critical: findings
            .iter()
            .filter(|finding| finding.severity() == Severity::Critical)
            .count(),
    }
}

fn print_human(report: &Report<'_>) {
    println!(
        "PAM Desktop permission audit · {}",
        report.application_identifier
    );
    println!(
        "Critical {} · High {} · Warning {} · Info {}\n",
        report.counts.critical, report.counts.high, report.counts.warning, report.counts.info
    );
    if report.findings.is_empty() {
        println!("[ok] No privileged capabilities are declared.");
        return;
    }
    for finding in report.findings {
        println!(
            "[{}] {} · {}\n  {}\n  Next: {}",
            finding.severity().label(),
            finding.rule,
            finding.resource,
            finding.message,
            finding.remediation,
        );
    }
}

#[cfg(test)]
mod tests {
    use pam_desktop_protocol::{FileSystemRootConfig, ProcessCommandConfig};

    use super::*;

    #[test]
    fn classifies_broad_writes_unpinned_plugins_and_dynamic_processes() {
        let root = std::env::temp_dir().join(format!(
            "pam-desktop-permission-audit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("storage")).expect("project root");
        let capabilities = NativeCapabilities {
            filesystem_roots: vec![
                FileSystemRootConfig {
                    name: "project".to_owned(),
                    path: root.to_string_lossy().into_owned(),
                    access: FileAccess::ReadWrite,
                },
                FileSystemRootConfig {
                    name: "data".to_owned(),
                    path: "storage".to_owned(),
                    access: FileAccess::ReadWrite,
                },
            ],
            clipboard_read: true,
            processes: vec![ProcessCommandConfig {
                name: "convert".to_owned(),
                executable: "bin/convert".to_owned(),
                arguments: Vec::new(),
                argument_policy: ProcessArgumentPolicy::Append,
            }],
            ..NativeCapabilities::default()
        };
        let mut findings = Vec::new();
        audit_capabilities(&root, &capabilities, &mut findings);
        audit_plugins(
            &[RustPluginConfig {
                id: "camera".to_owned(),
                executable: "bin/camera".to_owned(),
                arguments: Vec::new(),
                timeout_ms: 1_000,
                sha256: None,
            }],
            &mut findings,
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "filesystem.broad-write"
                    && finding.severity() == Severity::Critical)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "filesystem.write"
                    && finding.severity() == Severity::Warning)
        );
        assert!(findings.iter().any(
            |finding| finding.rule == "clipboard.read" && finding.severity() == Severity::High
        ));
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "process.dynamic-arguments"
                    && finding.severity() == Severity::High)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "plugin.unpinned"
                    && finding.severity() == Severity::Critical)
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parses_stable_audit_policy_and_origin_scope() {
        let options = parse(vec![
            "permissions".into(),
            "fixture".into(),
            "--json".into(),
            "--deny-high".into(),
        ])
        .expect("options");
        assert_eq!(options.project, Path::new("fixture"));
        assert!(options.json);
        assert_eq!(options.deny, Severity::High);
        assert_eq!(origin_base_path("https://api.example.com"), "/");
        assert_eq!(origin_base_path("https://api.example.com/v1"), "/v1");
        assert!(parse(vec!["unknown".into()]).is_err());
    }

    #[test]
    fn serializes_sequential_integer_severities_for_ci() {
        assert_eq!(Severity::Info as u8, 1);
        assert_eq!(Severity::Warning as u8, 2);
        assert_eq!(Severity::High as u8, 3);
        assert_eq!(Severity::Critical as u8, 4);
        let findings = vec![Finding::new(
            Severity::High,
            "clipboard.read",
            "clipboard",
            "Sensitive capability.",
            "Disable it.",
        )];
        let report = Report {
            schema_version: 1,
            surface_code: 3,
            result_code: 2,
            deny_severity_code: Severity::High as u8,
            application_identifier: "com.example.audit",
            counts: counts(&findings),
            findings: &findings,
        };
        let value = serde_json::to_value(report).expect("audit JSON");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["surfaceCode"], 3);
        assert_eq!(value["resultCode"], 2);
        assert_eq!(value["denySeverityCode"], 3);
        assert_eq!(value["counts"]["high"], 1);
        assert_eq!(value["findings"][0]["severityCode"], 3);
        assert!(value["findings"][0].get("severity").is_none());
    }
}

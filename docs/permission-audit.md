# Permission audit

`doctor` proves that a PAM Desktop declaration is valid. The permission audit
answers a different release question: which valid capabilities deserve a human
security decision before this application ships?

```bash
pam desktop audit permissions
```

The default policy exits unsuccessfully only when a critical finding exists.
Use the stricter policy for a trusted release branch:

```bash
pam desktop audit permissions --deny-high
pam desktop audit permissions --deny-high --json > permission-audit.json
```

JSON uses schema `1`, Desktop surface code `3`, result code `1` for pass or `2`
for fail, and sequential severity codes:

| Code | Severity | Default outcome |
| ---: | --- | --- |
| `1` | Info | Pass |
| `2` | Warning | Pass |
| `3` | High | Pass; fail with `--deny-high` |
| `4` | Critical | Fail |

Every finding includes a stable rule identifier, bounded resource identity,
message and remediation. Findings are sorted by descending severity and then by
rule/resource, making the output deterministic for CI review. No filesystem
paths, clipboard values, secrets or HTTP payloads enter the report.

## Current rules

Critical findings:

- a writable filesystem root resolves to the whole project, one of its parents,
  or an external directory;
- a filesystem root cannot be resolved to a stable directory;
- a process-isolated Rust plugin executable has no SHA-256 pin.

High findings:

- broad or external readable filesystem roots;
- clipboard reads;
- operating-system secret storage;
- autostart behavior;
- process commands that accept renderer-supplied arguments.

Warnings cover scoped write roots, clipboard writes, user-mediated external
grants, system metadata, origin-wide native HTTP, fixed process execution,
pinned Rust plugins, background jobs and global shortcuts. An absent signed
update feed is informational because offline and externally managed deployments
remain legitimate.

The audit does not claim that a capability is malicious. It exposes authority
that is easy to miss during code review and provides a stable policy gate. Run
it after Composer installation and before packaging, against the exact release
application configuration.

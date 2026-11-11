# SSH

The `ssh` module keeps SSH sessions in the host: commands, SFTP, local port forwarding and interactive shells. The PHP API is the package `pushinbr/pam-desktop-ssh` ([README](../packages/ssh/README.md)); this page describes the module itself, for people who call it directly or build a native terminal on top of it.

It is built on [russh](https://github.com/warp-tech/russh) and [russh-sftp](https://github.com/AspectUnk/russh-sftp), with pure Rust key exchange and ciphers (ring, the same crate the WebSocket TLS already uses), so no OpenSSL is needed on any system. It is the cargo feature `ssh` of `pam-desktop-host`, on by default.

## Threads and idle

The module runs one tokio runtime: a `pam-ssh` thread that owns it, two `pam-ssh` workers and at most four `pam-ssh` blocking threads for local file work. It starts with the first `connect` and stops when the last session ends, so an application with no session open has no SSH thread. A module call is validated on the UI thread and answered from the runtime.

## Methods

Every payload is a WireMap. Byte values (`bytes`, `stdout`, `stderr`) use the opaque bytes tag (5); PHP sends them with `Pam\Desktop\Modules\WireBytes` and receives them as plain strings. A failure is a short message, never a host, user, secret, command, path or the server's own text.

| Method | Payload | Reply |
| --- | --- | --- |
| `connect` | `host`, `port` (22), `username`, `password`?, `privateKey`? (OpenSSH, PEM or PuTTY text), `passphrase`?, `expectedFingerprint`?, `timeoutMs` (15000; 1000–120000), `keepaliveSeconds` (30; 0 off, max 3600) | `id`, `fingerprint`, `algorithm` |
| `disconnect` | `id` | empty |
| `next` | `id` | one event (below) |
| `exec` | `id`, `command` (≤ 64 KiB), `timeoutMs` (30000; 100–600000), `maxBytes` (983040; stdout and stderr together) | `exitCode` (-1 when the server sent none), `stdout`, `stderr`, `truncated` |
| `sftpList` | `id`, `path` | `entries` (JSON list), `count`, `truncated` |
| `sftpStat` | `id`, `path`, `noFollow`? | `name`, `kind`, `size`?, `mtime`?, `permissions`?, `uid`?, `gid`? |
| `sftpMkdir` | `id`, `path`, `mode`? | empty |
| `sftpRename` | `id`, `from`, `to` | empty |
| `sftpRemove` | `id`, `path`, `recursive`? | empty |
| `sftpRead` | `id`, `path`, `maxBytes` (983040 at most) | `bytes`, `size`, `truncated` |
| `sftpWrite` | `id`, `path`, `bytes` (≤ 983040), `mode`? | `bytes` (count written) |
| `sftpRealpath` | `id`, `path` | `path` |
| `sftpUpload` | `id`, `localPath` (absolute), `remotePath`, `mode`? | `transfer` |
| `sftpDownload` | `id`, `remotePath`, `localPath` (absolute) | `transfer` |
| `sftpCancel` | `transfer` | empty |
| `forwardLocal` | `id`, `bindHost` (`127.0.0.1`), `bindPort` (0 picks one), `remoteHost`, `remotePort`, `allowNonLoopback`? | `forward`, `port` |
| `forwardStop` | `forward` | empty |
| `shellOpen` | `id`, `cols` (80), `rows` (24), `term` (`xterm-256color`) | `channel` |
| `shellWrite` | `channel`, `bytes` (≤ 64 KiB) | empty |
| `shellResize` | `channel`, `cols`, `rows` (1–1000) | empty |
| `shellClose` | `channel` | empty |

- **Host keys.** `fingerprint` is the SHA-256 of the server's key in unpadded base64, without `SHA256:` (what `ssh-keygen -lf` prints after it). With `expectedFingerprint` (the prefix and padding are accepted), another key fails with `host-key-mismatch:<new fingerprint>`; without it any key is accepted and returned, so the application stores it (trust on first use).
- **Authentication.** A key is tried first, then the password, then keyboard-interactive answered with the password. Failures start with `auth-failed`; a key that cannot be read is `invalid-key: …`. Connection failures are `connect-failed: refused|timeout|host not found|unreachable`, `handshake-failed`, `timeout`.
- **Paths.** A remote path starting with `~` is relative to the login directory. `sftpList` skips `.` and `..` and stops at 5000 entries or 900 KiB of JSON; each entry has `name`, `kind` (`file`, `dir`, `link`, `other`), `size`, `mtime`, `permissions` (octal text such as `0644`), `owner`, `group`, `uid`, `gid` (null when the server sent none; owner and group come from the long name on SFTP version 3 servers). `sftpRemove` with `recursive` deletes a tree without following links (at most 100,000 entries, 64 levels). SFTP failures are `sftp: no-such-file`, `sftp: permission-denied`, `sftp: failure`, `sftp: timeout`, `sftp: connection lost`….
- **Transfers.** Up to 16 requests of 32 KiB are in flight. A download is written to `<localPath>.part` and renamed when complete; a failed or cancelled one removes it.
- **Forwards** listen only on a loopback address (`127.0.0.1`, `::1`, `localhost`) unless `allowNonLoopback` is set.

Limits: 16 sessions open or connecting, and per session 8 commands running, 8 shells, 16 forwards, 4 transfers; 64 connections per forward. The SFTP subsystem is opened at the first SFTP call and reused by every later one.

## Events

`next` answers one event per call. Only one `next` may wait per session; it is parked in the host until an event arrives.

| `kind` | Event | Keys |
| --- | --- | --- |
| 1 | Shell data | `channel`, `bytes` |
| 2 | Shell exit | `channel`, `exitCode` (-1 when unknown) |
| 3 | Transfer progress | `transfer`, `bytes`, `total` |
| 4 | Transfer done | `transfer`, `bytes` |
| 5 | Transfer failed | `transfer`, `message` (`cancelled` after `sftpCancel`) |
| 6 | Forward opened | `forward`, `port` |
| 7 | Forward failed | `forward`, `message` |
| 8 | Forward connections | `forward`, `connections`, `failures` |
| 9 | Session closed | |
| 10 | Session failed | `message` |

The queue holds at most 256 events and 512 KiB of shell output. Shell output is coalesced per channel into events of at most 64 KiB, and when PHP falls behind the shell's reader waits instead of growing the queue (the server then waits too). Progress (at most ten a second) and connection counts replace their previous queued value. Any other event that does not fit ends the session with `Session failed`: events are never dropped silently. After `Session closed` or `Session failed` the next `next` fails.

## Terminals without PHP

A shell's output does not have to pass through PHP. `crates/pam-desktop-ui/src/modules/terminal.rs` holds the process-wide `TerminalHub` (`TerminalHub::shared()`), a registry of terminal channels by id; the `channel` that `shellOpen` returns is that id. The render core can reach it, so a native terminal element can be built there:

```rust
use pam_desktop_ui::modules::terminal::{TerminalEvent, TerminalHub};

let hub = TerminalHub::shared();
let mut subscription = hub.subscribe(&channel, Arc::new(move |event| match event {
    // Called on a pam-ssh thread: post the bytes to the UI thread's inbox.
    TerminalEvent::Output(bytes) => inbox.post(/* … */),
    TerminalEvent::Exited(status) => inbox.post(/* … */),
}))?;
let backlog = subscription.take_backlog(); // recent output, at most 128 KiB
hub.write(&channel, b"ls\r");               // keystrokes
hub.resize(&channel, 120, 40);              // cells
drop(subscription);                         // detaches; output goes to PHP again
```

- One subscriber per channel; while it is attached, shell output is not sent to PHP's `next`.
- The callback runs on the producer's thread and never under a lock of the hub; it must not block.
- `write` and `resize` queue the request (256 waiting at most) and answer `false` when the channel is gone or the queue is full.
- The hub keeps the last 128 KiB of each channel, so a terminal that attaches late, or again after a hot reload of its screen, can repaint.

## Testing

The unit tests run with the host's (`cargo test -p pam-desktop-host ssh`). The end-to-end test needs an OpenSSH server and is ignored by default:

```bash
ssh-keygen -q -t ed25519 -N '' -f /tmp/pam-ssh/id_ed25519
mkdir -p /tmp/pam-ssh/keys /tmp/pam-ssh/entry
cp /tmp/pam-ssh/id_ed25519.pub /tmp/pam-ssh/keys/pam
printf '#!/bin/sh\necho pam:pam-test-pass | chpasswd\nusermod -s /bin/sh pam\n' > /tmp/pam-ssh/entry/10-password
chmod +x /tmp/pam-ssh/entry/10-password
docker run -d --name pam-ssh-test -p 127.0.0.1:22922:22 \
  -e SSH_USERS=pam:1000:1000 -e SSH_ENABLE_PASSWORD_AUTH=true -e TCP_FORWARDING=true \
  -v /tmp/pam-ssh/keys:/etc/authorized_keys:ro -v /tmp/pam-ssh/entry:/etc/entrypoint.d:ro \
  panubo/sshd
PAM_SSH_TEST_PORT=22922 PAM_SSH_TEST_PASSWORD=pam-test-pass PAM_SSH_TEST_KEY=/tmp/pam-ssh/id_ed25519 \
  cargo test -p pam-desktop-host end_to_end_against_openssh -- --ignored
docker rm -f pam-ssh-test
```

It covers password and key authentication, a wrong password, a pinned and a mismatched host key, `exec` (exit code, both streams, the output cap, the timeout), every SFTP call, an upload and a download of 3 MiB with progress, a failed download, a local forward to the server's own sshd, and a shell read through `next` and then through the hub, with a resize checked by `stty size`.

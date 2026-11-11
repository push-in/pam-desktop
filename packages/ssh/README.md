# PAM Desktop SSH

SSH sessions for PAM Desktop applications: remote commands, SFTP with uploads and downloads, local port forwarding and interactive shells. Connections, encryption and transfers run in the host (russh, pure Rust crypto, no OpenSSL) on threads that exist only while a session is open; PHP hears results and events and never polls.

```php
use Pam\DesktopSsh\{ExecResult, HostKeyMismatch, Shell, SftpListing, Ssh, SshException, SshOptions, SshSession};

$this->session = Ssh::connect(
    SshOptions::password('server.example', 'deploy', $password, expectedFingerprint: $this->savedFingerprint),
    function (SshSession $session): void {
        $this->savedFingerprint ??= $session->fingerprint(); // trust on first use

        $session->exec('uptime', function (ExecResult $result): void {
            $this->uptime = trim($result->stdout);
        });

        $session->sftp()->list('~', function (SftpListing $listing): void {
            $this->files = $listing->entries;
        });

        $session->shell(function (Shell $shell): void {
            $shell->onData(fn (string $bytes) => $this->terminal .= $bytes)
                ->onExit(fn (?int $status) => $this->shellOpen = false);
            $shell->write("ls -la\n");
        }, cols: 120, rows: 40);
    },
    function (SshException $error): void {
        $this->error = $error instanceof HostKeyMismatch
            ? 'The server key changed to '.$error->fingerprint
            : $error->getMessage();
    },
    owner: $this,
);
```

## API

| Call | Meaning |
| --- | --- |
| `Ssh::connect($options, $connected, $failed, $owner)` | Connects and authenticates. With an `$owner` component the session is disconnected when it unmounts. `$failed` receives `HostKeyMismatch` (with `->fingerprint`), `AuthFailed` or `SshException`. |
| `SshOptions` | `host`, `username`, `port`, `password`, `privateKey` (OpenSSH or PEM text), `passphrase`, `expectedFingerprint`, `timeoutMs`, `keepaliveSeconds`. `SshOptions::password()` and `SshOptions::key()` for the usual cases. Secrets are hidden from `var_dump()`. |
| `$session->fingerprint()`, `algorithm()` | The server's host key: SHA-256 in unpadded base64 without `SHA256:`, and its algorithm. Store the fingerprint and pass it back as `expectedFingerprint`. |
| `$session->exec($command, $done, $failed, $timeoutMs, $maxBytes)` | `ExecResult`: `exitCode`, `stdout`, `stderr` (raw bytes), `truncated`. |
| `$session->sftp()` | `SftpClient`: `list`, `stat`, `mkdir`, `rename`, `remove` (optionally recursive), `read` (the start of a file, up to 960 KiB), `write`, `realpath` (`~` is the login directory), `upload` and `download` returning a `Transfer` (progress at most ten times a second, `cancel()`). |
| `$session->forward($bindPort, $remoteHost, $remotePort, $opened)` | `ssh -L`: a `PortForward` with `port()` (0 picks a free one), connection counts and `stop()`. Loopback only unless `allowNonLoopback: true`. |
| `$session->shell($opened, $failed, $cols, $rows, $term)` | `Shell`: `onData`, `onExit`, `write`, `resize`, `close`, and `channel()`, the id a native terminal element binds to. |
| `$session->onEvent($listener)`, `onClose($listener)` | Every raw `SshEvent`; the end of a session that the server closed or that failed (a failure without `onClose` is thrown). |
| `$session->disconnect()`, `dispose()` | Ends the session, its shells, transfers and forwards. |

Every method with a `$failed` closure throws the `SshException` from its callback when the closure is omitted. Failure messages are short codes (`auth-failed`, `connect-failed: refused`, `exec-timeout`, `sftp: no-such-file`, `cancelled`…) and never contain the host, the user, a secret or a path.

## Terminal output

When a native terminal element is attached to `Shell::channel()` (through the host's terminal hub), the output goes straight to it and `onData()` is no longer called; keystrokes can be sent by the element as well. Without one, output arrives in `onData()` as raw bytes, in order, coalesced into chunks of at most 64 KiB. A UTF-8 character or an escape sequence may be split between two chunks.

## Limits

16 sessions; per session 8 commands at once, 8 shells, 16 forwards and 4 transfers. Commands, writes and reads are bounded by the 1 MiB module message (960 KiB of bytes); listings by 5000 entries. Methods, events and limits in full: [SSH](../../docs/ssh.md).

Validation: `php packages/ssh/tests/run.php` from the framework repository, and the host's Rust tests, including an end-to-end test against OpenSSH in a container (see the docs).

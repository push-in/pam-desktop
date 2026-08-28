# Application lifecycle

Pam Desktop applications are single-instance by default on Linux, macOS, and
Windows. The primary process owns an operating-system local IPC endpoint derived
from the reverse-DNS application ID. A second launch forwards at most 64 KiB of
arguments, waits for a protocol acknowledgement, and exits only after the
primary confirms delivery. The primary renderer receives
`pam.lifecycle.second-instance`; arguments from the initial desktop activation
arrive as `pam.lifecycle.opened`.

Declare desktop integration in PHP:

```php
use Pam\Desktop\Lifecycle;

$app->lifecycle(
    Lifecycle::none()
        ->schemes('myapp')
        ->files('application/x-myapp-document')
        ->autostart(),
);
```

The Linux package emits `MimeType` entries for normal MIME types and
`x-scheme-handler/<scheme>`, adds `%U` to the desktop command, refreshes the
desktop database during portable installation, and installs an autostart entry
only when explicitly enabled. Uninstall removes that entry. The same forwarded
argument channel handles deep links and associated files; application code must
still validate every received URI or path before use.

Installed quick actions travel through a reserved
`--pam-quick-action=<declared-id>` launcher argument. The host removes that
internal argument, verifies it against the signed shell declaration, and emits
`pam.quick-action.selected`; it never exposes the reserved transport flag to
application lifecycle handlers.

On Linux, the Unix socket is mode `0600` and confined to `XDG_RUNTIME_DIR`. If
that directory is unavailable, PAM creates and verifies an owner-only `0700`
runtime directory instead of placing the socket directly in the shared temp
directory. A stale socket is replaced only after connection to a live primary
fails. macOS and Windows use namespaced local sockets (Unix local socket or
Windows named pipe) rather than TCP, so activation never leaves the machine or
opens a listening network port. Every platform bounds and strictly decodes the
activation envelope.

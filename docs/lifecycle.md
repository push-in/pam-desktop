# Linux lifecycle

Pam Desktop applications are single-instance by default. The primary process
owns a per-user Unix-domain socket derived from the reverse-DNS application ID.
A second launch forwards at most 64 KiB of arguments and exits; the primary
renderer receives `pam.lifecycle.second-instance`. Arguments from the initial
desktop activation arrive as `pam.lifecycle.opened`.

Declare Linux desktop integration in PHP:

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

The socket is confined to `XDG_RUNTIME_DIR` when available, never accepts more
than the bounded JSON activation envelope, and is removed by the owning process.
A stale socket is replaced only after connection to a live primary fails.

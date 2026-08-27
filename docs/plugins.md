# Plugins

Pam Desktop 1.0 supports two extension styles with different trust boundaries:

- PHP plugins compose application policy inside the supervised Pam worker;
- Rust plugins run as separate supervised executables and expose explicit
  commands to the trusted frontend.

Neither style receives an unrestricted browser bridge. PHP plugins use the
public `Application` API, while Rust plugins communicate through a bounded,
versioned JSON-lines protocol.

## PHP plugins

A PHP plugin is a small composition object. Its stable identifier is included
in the boot contract and its `register()` method may add commands, events,
background jobs, windows, or other public application configuration.

```php
<?php

declare(strict_types=1);

namespace App;

use Pam\Desktop\Application;
use Pam\Desktop\CommandContext;
use Pam\Desktop\Plugin;

final class RuntimePlugin implements Plugin
{
    public function identifier(): string
    {
        return 'runtime';
    }

    public function register(Application $application): void
    {
        $application->command(
            'runtime.snapshot',
            static fn (CommandContext $command): array => [
                'php' => PHP_VERSION,
                'window' => $command->windowId,
            ],
        );
    }
}
```

Register it before `run()`:

```php
$app->plugin(new \App\RuntimePlugin());
```

Plugin identifiers and every registered command remain unique. If registration
throws, Pam removes the incomplete plugin declaration and fails boot.

## Rust plugin scaffold

Create a process-isolated plugin inside an application:

```bash
pam desktop plugin new system.info
pam desktop plugin build system.info
```

The first command creates `plugins/system.info`; the second performs a locked
release build when a lockfile exists and copies the executable to
`plugins/bin/system.info` on Linux. The scaffold pins the SDK to the matching
Pam Desktop tag.

Declare the resulting project-relative executable in PHP:

```php
use Pam\Desktop\RustPlugin;

$app->rustPlugin(
    RustPlugin::executable('system.info', 'plugins/bin/system.info')
        ->integrity('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef')
        ->timeout(5_000),
);
```

Call an exported command from the trusted frontend:

```js
const controller = new AbortController();

const snapshot = await window.pam.plugins.invoke(
    "system.info",
    "hello",
    { detail: "short" },
    { timeout: 5_000, signal: controller.signal },
);
```

## Rust SDK

The generated executable implements `pam_desktop_plugin::Plugin`:

```rust
use pam_desktop_plugin::protocol::PluginMetadata;
use pam_desktop_plugin::{
    Plugin, PluginContext, PluginFailure, PluginOutput, serve,
};

struct SystemInfoPlugin;

impl Plugin for SystemInfoPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            identifier: "system.info".to_owned(),
            name: "System Info".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commands: vec!["snapshot".to_owned()],
        }
    }

    fn invoke(
        &mut self,
        context: PluginContext,
    ) -> Result<PluginOutput, PluginFailure> {
        match context.command.as_str() {
            "snapshot" => Ok(PluginOutput::new(serde_json::json!({
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
            }))),
            _ => Err(PluginFailure::handler_failed("Unknown command.")),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    serve(SystemInfoPlugin)?;
    Ok(())
}
```

Registration validates the project-relative executable and its optional hash
without spawning it. The process and protocol handshake are initialized lazily
on the plugin's first invocation; metadata is then validated and pinned for
every recovery. Declared identity and exported commands cannot change while the
application is running. Applications with many Composer-discovered extensions
therefore pay no process or handshake cost for unused plugins.

## Isolation and failure semantics

Rust extensions are executables, not dynamic libraries. This avoids loading an
unstable or unsafe ABI into the Servo host and gives every plugin a separate
process boundary.

That process boundary provides crash containment and a stable transport; it is
not an operating-system sandbox. A Rust plugin is trusted native code with the
ambient authority of the application user. Only install and register plugins
you trust, and model sensitive access through narrow plugin commands.

For each configured plugin, the host:

1. resolves a regular executable below the project while rejecting `.git`,
   `.pam`, `dist`, `node_modules`, `target`, parent traversal, and symlinks;
2. verifies the optional pinned SHA-256 at registration, first use and every restart;
3. starts the process lazily with an empty inherited environment and piped standard
   input/output, exposing only documented `PAM_DESKTOP_*` variables;
4. performs a protocol-1 boot handshake and validates its exact metadata;
5. serializes calls per plugin, enforces the declared export list, one-megabyte
   message limit, timeout, and cancellation;
6. terminates a timed-out, cancelled, malformed, or crashed process;
7. prepares a fresh process for the next call without replaying the interrupted
   command.

The no-replay rule is intentional: a failed command may already have performed
an external side effect. Plugin events enter the same ordered frontend event
hub as PHP events.

## Packaging

Only configured plugin executables are required at application boot. Keep
sources under `plugins/<id>` and built executables under `plugins/bin`; the
normal application bundle excludes the standard scaffold sources, copies the
configured executable and verifies that every declared executable is present.

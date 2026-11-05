# Signed updates

Updates are opt-in. The application pins an HTTPS endpoint, a channel, an
Ed25519 public key and a policy in `pam-desktop.json`; the private signing key
never ships with the application. A release is accepted only when its feed was
signed by the holder of that key and the downloaded archive has the signed
size and SHA-256.

## Manifest

```json
{
    "applicationId": "com.example.my-app",
    "version": "1.0.0",
    "updates": {
        "endpoint": "https://updates.example.com/my-app/stable.json",
        "publicKey": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "channel": "stable",
        "policy": "notify"
    }
}
```

| Field | Default | Rules |
| --- | --- | --- |
| `endpoint` | required | HTTPS URL of the feed, without credentials or fragment, at most 2048 bytes |
| `publicKey` | required | the 32-byte Ed25519 public key as 64 lowercase hexadecimal characters |
| `channel` | `stable` | starts with a letter; at most 64 letters, digits, `.`, `-`, `_` |
| `policy` | `manual` | `manual`, `notify` or `automatic` |

| Policy | Behavior |
| --- | --- |
| `manual` | nothing happens until the application calls `Updates::check()` |
| `notify` | the host checks the feed when the application starts and reports what it found |
| `automatic` | the host checks, downloads, verifies, installs and restarts the application |

Without `updates` the state is `disabled` and every operation is refused. An
invalid `updates` block stops `pam-desktop build` and `publish-update`; a
running application reports the problem on standard error and runs with
updates disabled rather than with a feed it did not mean to trust. Other
fields are not accepted inside `updates`.

The `notify` and `automatic` policies act when the application starts, outside
`pam-desktop dev`: a development run is not an installed bundle, so it only
checks when the application asks.

The feed, the artifact and every redirect on the way to them (at most three)
must be HTTPS. Debug builds of the host also accept plain HTTP to the local
machine, for integration tests.

## Signing keys

Generate a key once, on an offline or tightly controlled release machine:

```bash
pam-desktop update-key --output /secure/my-app-update.key
```

The command creates a 32-byte Ed25519 seed as lowercase hexadecimal, with mode
`0600` on Unix, and refuses to overwrite a file. It prints only the public
key: put that value in `updates.publicKey` and keep the file in a secret
manager. `publish-update` refuses a key file that its group or others can read
and a key whose public half is not the one of the manifest.

## Feed publication

Build the artifacts on their systems (`pam-desktop build --format portable`),
collect them in the trusted release job, then sign one feed with every target:

```bash
pam-desktop publish-update path/to/app \
  --key /secure/my-app-update.key \
  --output dist/stable.json \
  --published-at 2026-07-23T14:00:00Z \
  --notes-url https://example.com/releases/1.0.0 \
  --artifact linux,x86_64,portable,dist/my-app-1.0.0-linux-x86_64.tar.gz,https://cdn.example.com/my-app-1.0.0-linux-x86_64.tar.gz \
  --artifact macos,aarch64,portable,dist/my-app-1.0.0-macos-aarch64.zip,https://cdn.example.com/my-app-1.0.0-macos-aarch64.zip
```

The application (the current directory by default) provides the identifier and
the version of the release. `--channel` defaults to `stable`; `--key`,
`--output`, `--published-at` and at least one `--artifact` are required. Each
`--artifact` is:

```text
platform,architecture,kind,local-path,HTTPS-URL
```

Platforms are `linux`, `macos` and `windows`; the architecture is the one in
the name of the artifact (`x86_64`, `aarch64`); kinds are `portable` and
`installer`. The updater installs portable archives; installers can be listed
for download pages. The publisher computes byte lengths and SHA-256 values
itself, writes the feed through a staging file in the same directory, and
refuses an existing destination without `--force`.

```json
{
  "release": {
    "schemaVersion": 1,
    "applicationId": "com.example.my-app",
    "channel": "stable",
    "version": "1.0.0",
    "publishedAt": "2026-07-23T14:00:00Z",
    "notesUrl": "https://example.com/releases/1.0.0",
    "artifacts": [
      {
        "platform": 1,
        "architecture": "x86_64",
        "kind": 1,
        "url": "https://cdn.example.com/my-app-1.0.0-linux-x86_64.tar.gz",
        "bytes": 22916595,
        "sha256": "64-lowercase-hex-characters"
      }
    ]
  },
  "signature": "128-lowercase-hex-characters"
}
```

Platforms are the integers `1` (Linux), `2` (Windows) and `3` (macOS); kinds
are `1` (portable archive) and `2` (installer). The signature covers the
compact JSON serialization of `release` in the field order above (`notesUrl`
left out when absent). Unknown fields are rejected. A feed is at most one MiB.

The updater verifies the signature strictly, then requires the identifier and
the channel of the application, a version newer than the running one (numeric
parts compared as numbers, a release above its pre-releases, build metadata
ignored) and a portable artifact for the running platform and architecture.

## PHP API

```php
use Pam\Desktop\Updates;
use Pam\Desktop\UpdateState;

Updates::onChange(function (UpdateState $state): void {
    match ($state->status) {
        UpdateState::AVAILABLE => Updates::download(),
        UpdateState::DOWNLOADING => $this->progress = $state->progress(),
        UpdateState::READY => $this->canRestart = true,
        UpdateState::FAILED => $this->error = $state->error,
        default => null,
    };
});

Updates::check();

// From a button, once the state is ready:
Updates::install();
```

| Method | Effect |
| --- | --- |
| `Updates::state(Closure $completed)` | reads the current `UpdateState` |
| `Updates::check(?Closure $completed = null)` | asks the feed for a newer release |
| `Updates::download(?Closure $completed = null)` | downloads and verifies the available release |
| `Updates::install(?Closure $completed = null)` | applies the downloaded release: the application closes and starts again |
| `Updates::onChange(Closure $listener): int` | runs the listener on every change; returns a subscription |
| `Updates::offChange(int $subscription)` | removes the listener |

`UpdateState` is immutable: `status`, `currentVersion`, `availableVersion`,
`downloadedBytes`, `totalBytes`, `notesUrl`, `error`, `policy`, with
`isEnabled()`, `isAvailable()`, `isReady()`, `isFailed()`, `isBusy()` and
`progress()` (0 to 1). The statuses are the constants `DISABLED`, `IDLE`,
`CHECKING`, `AVAILABLE`, `DOWNLOADING`, `READY`, `APPLYING`, `UP_TO_DATE` and
`FAILED`.

An operation that is refused (no release to download, another operation
running, a feed that does not verify) completes with a `FAILED` state carrying
the reason. Listeners see every change, whoever caused it: a call from the
application or the policy working by itself. During a download they receive one
change per percent.

The native module behind this API is `updates`, with the methods `state`,
`check`, `download`, `install` and the long poll `next`. Operations run one at
a time on a worker thread of the host, never on the interface thread.

## Installation

Downloads are staged beside the installed bundle, so the swap is a rename on
one file system; they are bounded by the signed byte length and must match
both the length and the SHA-256 of the feed.

`install` copies the host into the staging directory and starts it as a
helper, then the application exits. The helper:

1. verifies the archive against the signed SHA-256 again;
2. waits for the application to exit (up to two minutes);
3. extracts the archive, which must hold exactly one bundle directory;
4. verifies every file of the new bundle against its `manifest.json`, and that
   it is the same application as the installed one (on macOS also the code
   signature of the `.app`, so only applications built with `--sign` update);
5. moves the installed bundle to `.<name>.previous`, moves the new one into
   place and verifies it again, rolling back if anything fails;
6. starts the launcher of the new bundle.

Only an application that runs from a bundle it may replace can be updated: the
portable bundle (extracted, or installed with `install.sh`) and the macOS
`.app`. A development run, a system package below `/opt`, an AppImage or an
MSIX report a failure at `download` or `install`; their package managers
update them. On Linux the helper needs `tar`; on macOS `ditto`; on Windows
PowerShell.

## Interrupted installation recovery

Immediately before the first rename, the helper writes a journal beside the
bundle, bound to the SHA-256 of the verified archive. Running the same
installation again reconciles the file system instead of repeating the swap
blindly:

- a missing bundle with a valid `.previous` bundle restores the previous
  version and starts over;
- valid current and previous bundles mean the swap completed before the helper
  stopped: the installation finishes without consuming the rollback version;
- an invalid current bundle is moved to the single `.failed` quarantine slot
  once `.previous` verifies, and the previous version is restored;
- missing current and previous bundles, symbolic links, a malformed or
  oversized journal, or a journal of another archive fail without deleting
  anything.

A successful installation removes the archive and the extraction directory.
The helper itself stays in its staging directory, because a running program
cannot always be deleted; the next download keeps at most one earlier staging
directory before creating its own. `.previous` and `.failed` are single fixed
paths, not a history.

This covers a helper that stops at any point of the swap. Durability on power
loss still depends on the file system: test abrupt termination on the systems
you support.

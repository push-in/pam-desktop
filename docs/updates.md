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
and `expiresAtUnix` left out when absent). Unknown fields are rejected. A feed
is at most one MiB.

The updater verifies the signature strictly, then requires the identifier and
the channel of the application, a version newer than the running one (numeric
parts compared as numbers, a release above its pre-releases, build metadata
ignored) and a portable artifact for the running platform and architecture.

### Feed freshness

A signature proves who published a feed, not when. Two checks keep a mirror or
an attacker on the network from serving an old signed feed indefinitely:

- **Expiry (opt-in).** `publish-update --expires-in DAYS` (1 to 366) adds a
  signed `"expiresAtUnix"` (Unix seconds) as the last field of `release`. The
  updater refuses the feed once that moment has passed, so republish (re-sign)
  the feed before it expires, for example from a scheduled job. Feeds without
  the field verify exactly as before. Hosts older than this version reject
  feeds that carry it (unknown field), so add it only once every installed
  application runs a host that knows it.
- **No going back (always).** For an application that runs from a bundle, the
  updater records the newest version a verified feed of each channel offered
  in `.<name>.update-feed-seen` beside the bundle, and refuses a later feed of
  the same channel that offers an older version ("refusing a replayed feed").
  A feed that offers the same version again is fine. Changing channel starts a
  new record; an unreadable record is ignored, never trusted.

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

Every failure after the application closed for the update (an archive that
does not match, a bundle whose files do not match its `manifest.json`, another
application, a swap that had to roll back) ends the same way: the installed
version, or the one the rollback restored, starts again, the staged archive
and extraction are removed, and the SHA-256 of the refused archive is written
beside the bundle in `.<name>.rejected-update`. The `automatic` policy does
not download that exact archive again, so a broken release cannot close the
application at every start; it stays `AVAILABLE`, and `Updates::download()`
and `Updates::install()` can still try it on request. A successful
installation removes the record.

The host leaves for the helper with `_exit` on Linux and macOS, from the
updates worker: the exit handlers of GTK and of the GL driver do not run while
the interface thread still draws.

Only an application that runs from a bundle it may replace can be updated: the
portable bundle (extracted, or installed with `install.sh`) and the macOS
`.app`. A development run, a system package below `/opt`, an AppImage or an
MSIX report a failure at `download` or `install`; their package managers
update them. Nothing is taken from `PATH`: on Linux the archive is unpacked in
Rust (regular files, folders and relative symlinks that stay inside the
archive; no owners, setuid bits, extended attributes, hard links or device
files); on macOS the helper runs `/usr/bin/ditto` (and `/usr/bin/codesign`); on
Windows the PowerShell and `tasklist.exe` of `%SystemRoot%\System32`.

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

## Verified on Linux

Release host on Ubuntu 24.04 (x86_64), 2026-11-08: a real application built
as 0.1.0, 0.1.1 and 0.1.2 with `pam-desktop build --format portable`,
installed per user with `install.sh`, policy `automatic`, the feed signed by
`publish-update` and served over HTTPS by a local server whose certificate
authority was added to the system store (release hosts refuse plain HTTP,
even on the local machine).

| Feed or artifact | Result |
| --- | --- |
| signature with one byte changed | refused at `check`; no download; the application keeps running |
| `release` changed after signing | refused at `check` |
| signed with another key | refused at `check` |
| signed archive replaced on the server | download refused (size and SHA-256); nothing installed |
| signed archive of a bundle changed after the build | downloaded, refused by the helper (file does not match `manifest.json`); 0.1.0 started again, staged archive and extraction removed, the release recorded in `.<name>.rejected-update`; the next start checks the feed and does not download it again |
| valid 0.1.1 | checked, downloaded, verified, swapped and restarted in about 1.1 s; 0.1.0 kept as `.<name>.previous`; the restarted 0.1.1 checks once and is up to date |
| an older release after a newer one was offered | refused as a replay (the feed watermark): an application that saw 0.1.2 does not take 0.1.1 |

The feed is fetched once per start and never again: no request and no CPU
time (0 ticks in 60 s) while the window was hidden, and none after it was
shown again. The `.deb` installation below `/opt` does not update itself, as
described above.

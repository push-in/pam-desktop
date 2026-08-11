# Signed updates

Pam Desktop updates are opt-in application policy. PHP pins an HTTPS endpoint,
channel, Ed25519 public key and integer-backed policy; the private signing seed
never ships with the application.

## PHP policy

```php
use Pam\Desktop\UpdatePolicy;
use Pam\Desktop\Updates;

$manifest = Manifest::create('com.example.my-app', 'My application', '1.0.0')
    ->updates(
        Updates::from(
            'https://updates.example.com/my-app/stable.json',
            '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
        )
            ->channel('stable')
            ->policy(UpdatePolicy::Notify),
    );
```

Policies are transmitted as sequential integers:

| Value | Policy | Behavior |
| --- | --- | --- |
| `1` | Manual | only explicit frontend calls |
| `2` | Notify | check in the background and emit state events |
| `3` | Automatic | check, download, verify, install and restart |

Omitting `updates()` disables every update route. Remote endpoints and artifact
URLs require HTTPS. Debug hosts also accept loopback HTTP for local integration
tests; redirects from HTTPS remain HTTPS-only.

## Signing keys

Generate a key once on an offline or tightly controlled release machine:

```bash
pam desktop update-key --output /secure/my-app-update.key
```

The command creates a 32-byte Ed25519 seed as lowercase hexadecimal with mode
`0600` on Unix. It prints only the public key. Put that public value in
`Updates::from()` and store the private file in a secret manager. The publisher
refuses group/world-readable Unix key files and refuses a key whose public half
does not match the PHP manifest.

## Feed publication

Build artifacts on their native hosts, collect them in the trusted release job,
then sign one feed containing every target:

```bash
pam desktop publish-update \
  --key /secure/my-app-update.key \
  --output dist/stable.json \
  --published-at 2026-07-23T14:00:00Z \
  --notes-url https://example.com/releases/1.0.0 \
  --artifact linux,x86_64,portable,dist/app-linux.tar.gz,https://cdn.example.com/app-linux.tar.gz
```

Each `--artifact` tuple is:

```text
platform,architecture,kind,local-path,HTTPS-URL
```

The 1.x supported release target is `linux,x86_64`; kinds are `portable` and
`installer`. Historical Windows/macOS feed parsing remains in the codebase but
is outside current artifact generation and support. The publisher calculates
byte lengths and SHA-256 values itself. It writes the feed through a
same-directory staging file and refuses an existing destination unless
`--force` is explicit.

The signed payload is the compact JSON serialization of the typed `release`
object in protocol field order. Unknown feed fields are rejected. The outer
document contains only:

```json
{
  "release": {
    "schemaVersion": 1,
    "applicationId": "com.example.my-app",
    "channel": "stable",
    "version": "1.0.0",
    "publishedAt": "2026-07-23T14:00:00Z",
    "artifacts": []
  },
  "signature": "128-lowercase-hex-characters"
}
```

The updater matches application ID, channel, newer version, integer platform,
exact architecture and portable artifact kind only after strict Ed25519
verification.

## Frontend API

```js
const current = await window.pam.updater.status();
const checked = await window.pam.updater.check();

if (checked.state === 4) { // Available
    const ready = await window.pam.updater.download();
    if (ready.state === 6) { // Ready
        await window.pam.updater.install();
    }
}
```

Lifecycle states are integer-backed: disabled `1`, idle `2`, checking `3`,
available `4`, downloading `5`, ready `6`, applying `7`, up-to-date `8`, and
failed `9`. Applications can also subscribe to `pam.update.changed`,
`pam.update.ready`, `pam.update.applying`, and `pam.update.error`.

Every bridge call repeats origin, ephemeral token and source-window checks.
Downloads run outside Tokio workers, are bounded by the signed byte length, and
must match both length and SHA-256. Installation uses a copied helper, waits for
the host process, verifies the extracted bundle, swaps within one filesystem,
retains the previous version for rollback and relaunches the declared launcher.

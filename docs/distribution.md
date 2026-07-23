# Multi-platform distribution

Pam Desktop 0.5 packages the application policy declared in PHP. The build
command boots the application, validates protocol 5, and derives identity,
version, publisher, category, icon and update policy from that typed contract.
It does not require a Node-based bundler.

## Application manifest

```php
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\Manifest;

$manifest = Manifest::create(
    identifier: 'com.example.my-app',
    name: 'My application',
    version: '0.5.0',
)
    ->description('A focused desktop tool managed in PHP.')
    ->publisher('Example')
    ->category(ApplicationCategory::Productivity)
    ->icon('resources/icon.png')
    ->excludeFromBundle('storage/cache', 'storage/development.sqlite');
```

Identifiers are lowercase reverse-DNS names of at most 155 bytes. Versions use
a portable numeric-leading format. Icons must be project-relative square PNG
or self-contained SVG files below `resources`; MSIX specifically requires PNG
because Windows package assets are rendered at several exact sizes.

Required application paths cannot be excluded. Pam also omits `.git`, `.pam`,
`.env*`, `dist`, `node_modules`, `target`, `.pamignore` and nested development
`vendor` directories from Composer path packages. Absolute `type=path`
repository locations are removed after their packages are materialized.

## Build formats

```bash
# Every platform: unpacked bundle + update-ready portable archive
pam desktop build .

# Linux
pam desktop build . --format deb

# macOS: signed/notarized DMG; Windows: signed MSIX
pam desktop build . --format native --sign

# Every format supported by the current host
pam desktop build . --output artifacts --format all
```

Existing destinations cause a failure. Pass `--force` only to replace those
exact versioned artifacts.

| Host | `directory` | `portable` | native installer |
| --- | --- | --- | --- |
| Linux | self-contained directory | deterministic `.tar.gz` | `.deb` |
| macOS | runtime directory | signed `.app` in `.zip` | `.dmg` |
| Windows | self-contained directory | `.zip` | `.msix` |

Packages must be built on their target operating system. The build copies the
host, the native launcher, Pam worker, application, isolated `php.ini`, adjacent
runtime libraries and validated product metadata. The launcher sets
`PAM_BINARY`, the bundle/update roots and the platform library path before
starting production mode.

Linux portable bundles also include atomic per-user `install.sh` and
`uninstall.sh` scripts. Debian packages install below `/opt`, expose a launcher
under `/usr/bin`, and install Freedesktop and hicolor metadata.

macOS packages use `Info.plist`, a hardened-runtime executable and a signed
`.app`. The portable ZIP is the update artifact; the DMG is the interactive
installer. Windows packages use a full-trust MSIX manifest and exact generated
Store, 44px and 150px assets.

## Platform signing

Signing credentials stay outside PHP and the repository.

macOS:

```bash
export PAM_MACOS_SIGNING_IDENTITY='Developer ID Application: Example (TEAMID)'
export PAM_MACOS_NOTARY_PROFILE='pam-notary' # optional keychain profile
pam desktop build . --format all --sign
```

`codesign` signs runtime binaries and the final application. When
`PAM_MACOS_NOTARY_PROFILE` is present, `xcrun notarytool` submits the DMG and
`stapler` attaches the accepted ticket.

Windows:

```powershell
$env:PAM_WINDOWS_CERTIFICATE_SHA1 = '40_HEX_DIGIT_STORE_THUMBPRINT'
$env:PAM_WINDOWS_PUBLISHER = 'CN=Example'
$env:PAM_WINDOWS_TIMESTAMP_URL = 'http://timestamp.digicert.com'
pam desktop build . --format all --sign
```

The certificate must already be installed in the signing user's certificate
store. `signtool.exe` signs each executable/DLL and the final MSIX using SHA-256
and an RFC 3161 timestamp. The manifest publisher must exactly match the
certificate subject.

## Integrity and compatibility

Every runtime bundle contains `manifest.json` with:

- schema and protocol versions;
- complete typed PHP application metadata;
- Pam Desktop and Pam runtime versions;
- operating system, architecture and ABI;
- every shipped relative path, byte count and SHA-256 digest.

The updater verifies the signed archive hash first and every extracted runtime
file before swapping. Signed macOS application updates additionally pass
`codesign --verify --deep --strict`. Archive installation keeps one previous
bundle and rolls back on swap or post-install verification failure.

Linux portable archives normalize ordering, ownership and timestamps using
`SOURCE_DATE_EPOCH` (zero by default). Build Linux artifacts on the oldest
supported glibc distribution. The release workflow builds tagged host binaries
on Ubuntu, Apple Silicon macOS and x86-64 Windows; product applications should
run their own signed package smoke tests on every declared target.

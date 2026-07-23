# Linux distribution

Pam Desktop 0.6 packages the application policy declared in PHP. The build
command boots the application, validates protocol 6, and derives identity,
version, publisher, category, icon and update policy from that typed contract.
It does not require a Node-based bundler.

## Application manifest

```php
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\Manifest;

$manifest = Manifest::create(
    identifier: 'com.example.my-app',
    name: 'My application',
    version: '0.6.0',
)
    ->description('A focused desktop tool managed in PHP.')
    ->publisher('Example')
    ->category(ApplicationCategory::Productivity)
    ->icon('resources/icon.png')
    ->excludeFromBundle('storage/cache', 'storage/development.sqlite');
```

Identifiers are lowercase reverse-DNS names of at most 155 bytes. Versions use
a portable numeric-leading format. Icons must be project-relative square PNG or
self-contained SVG files below `resources`.

Required application paths cannot be excluded. Pam also omits `.git`, `.pam`,
`.env*`, `dist`, `node_modules`, `target`, `.pamignore` and nested development
`vendor` directories from Composer path packages. Absolute `type=path`
repository locations are removed after their packages are materialized.

## Linux-first build formats

```bash
# Unpacked Linux bundle + update-ready portable archive
pam desktop build .

pam desktop build . --format deb
pam desktop build . --output artifacts --format all
```

Existing destinations cause a failure. Pass `--force` only to replace those
exact versioned artifacts.

| Host | `directory` | `portable` | native installer |
| --- | --- | --- | --- |
| Linux | self-contained directory | deterministic `.tar.gz` | `.deb` |

The official 0.6 release pipeline generates Linux x86-64 artifacts only. The
existing Windows/macOS packagers remain in the codebase for future work, but
they are not part of the current release or compatibility guarantee.

Linux packages are built on Linux. The build copies the
host, the native launcher, Pam worker, application, isolated `php.ini`, adjacent
runtime libraries and validated product metadata. The launcher sets
`PAM_BINARY`, the bundle/update roots and the platform library path before
starting production mode.

Linux portable bundles also include atomic per-user `install.sh` and
`uninstall.sh` scripts. Debian packages install below `/opt`, expose a launcher
under `/usr/bin`, and install Freedesktop and hicolor metadata.

Configured Rust plugin executables are copied with the application and must be
present before staging succeeds.

## Signing

Linux `.deb` and portable bundles use the per-file integrity manifest and the
separately signed update feed. Distribution signing secrets stay outside PHP
and the repository; see the updates guide for the Ed25519 publication flow.

## Integrity and compatibility

Every runtime bundle contains `manifest.json` with:

- schema and protocol versions;
- complete typed PHP application metadata;
- Pam Desktop and Pam runtime versions;
- operating system, architecture and ABI;
- every shipped relative path, byte count and SHA-256 digest.

The updater verifies the signed archive hash first and every extracted runtime
file before swapping. Archive installation keeps one previous bundle and rolls
back on swap or post-install verification failure.

Linux portable archives normalize ordering, ownership and timestamps using
`SOURCE_DATE_EPOCH` (zero by default). Build Linux artifacts on the oldest
supported glibc distribution. The release workflow builds tagged x86-64 host
binaries on Ubuntu 22.04. Product applications should run a clean install,
launcher and update smoke test on their oldest supported Linux distribution.

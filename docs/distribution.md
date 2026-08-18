# Linux distribution

Pam Desktop 1.0 packages the application policy declared in PHP. The build
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
    version: '1.0.0',
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
pam desktop build

pam desktop build --format deb
pam desktop build --output artifacts --format all
```

Existing destinations cause a failure. Pass `--force` only to replace those
exact versioned artifacts.

| Host | `directory` | `portable` | native installer |
| --- | --- | --- | --- |
| Linux | self-contained directory | deterministic `.tar.gz` | `.deb` |

The official 1.x release pipeline generates Linux x86-64 artifacts only. The
existing Windows/macOS packagers remain experimental code for future work;
they are not built, published, or covered by the 1.x compatibility guarantee.

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

- schema, public API and protocol versions;
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

## Official host archive

A tagged release builds the Servo host on Ubuntu 22.04, packages it twice with
the commit timestamp, and requires both archives to be byte-identical. The
archive contains:

- `bin/pam-desktop` and `bin/pam-desktop-launcher`;
- a schema-1 host `manifest.json` with public API `1`, protocol `6`, target,
  byte lengths and SHA-256 values;
- rootless `install.sh` and exact-version `uninstall.sh`;
- license and installation notes.

The adjacent `.sha256` file authenticates the complete compressed archive.
`scripts/test-host-archive.sh` rejects unsafe members, unexpected files,
manifest mismatches and non-reproducible or nonfunctional installs. It then
installs into temporary XDG directories, invokes the installed host and
uninstalls the exact version.

Before either a native-host or API-only GitHub Release can be created, the
release workflow calls the complete CI from the exact tagged commit. Formatting,
Clippy, workspace tests, Composer tests and static analysis, reproducible archive
contracts, footprint evidence and official Collector interoperability therefore
remain hard publication dependencies even when no native host files changed.

The release also publishes a sibling `.reproducibility.json` evidence manifest.
Schema `1`, suite `1`, Desktop surface `3` and result `1` mean that two
independent packaging directories produced the same bytes. The manifest binds
the archive name, size and SHA-256 to the source commit, `SOURCE_DATE_EPOCH`,
host OS and architecture. Both the archive and evidence receive GitHub artifact
attestations. CI verifies the manifest again before uploading it; pull-request
evidence uses a fixed artifact name and expires after 14 days.

The release also emits `.footprint.json` (schema `1`, suite `2`, Desktop surface
`3`). It authenticates the archive and records compressed, installed and host
executable byte counts without extracting untrusted paths. The performance gate
can compare this immutable record with the last accepted release and reject a
configurable percentage regression.

Reproduce the contract locally after building both host binaries:

```bash
SOURCE_DATE_EPOCH=0 scripts/package-host-linux.sh target/debug /tmp/host-one 1.2.1
SOURCE_DATE_EPOCH=0 scripts/package-host-linux.sh target/debug /tmp/host-two 1.2.1
SOURCE_DATE_EPOCH=0 scripts/host-reproducibility-evidence.sh create \
  /tmp/host-one/pam-desktop-1.2.1-x86_64-unknown-linux-gnu.tar.gz \
  /tmp/host-two/pam-desktop-1.2.1-x86_64-unknown-linux-gnu.tar.gz \
  /tmp/host-evidence/evidence-manifest.json
scripts/host-reproducibility-evidence.sh verify \
  /tmp/host-one/pam-desktop-1.2.1-x86_64-unknown-linux-gnu.tar.gz \
  /tmp/host-evidence/evidence-manifest.json
```

The installer defaults to
`${XDG_DATA_HOME:-$HOME/.local/share}/pam-desktop/<version>` and links commands
under `${XDG_BIN_HOME:-$HOME/.local/bin}`. It refuses to replace unrelated
commands and atomically changes only links already managed by PAM Desktop.

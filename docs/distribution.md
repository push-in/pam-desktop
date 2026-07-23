# Linux distribution

Pam Desktop 0.4 packages the application policy declared in PHP. The build
command never invents product identity from filenames and never requires a
Node-based bundler.

## Application manifest

```php
use Pam\Desktop\ApplicationCategory;
use Pam\Desktop\Manifest;

$manifest = Manifest::create(
    identifier: 'com.example.my-app',
    name: 'My application',
    version: '0.4.0',
)
    ->description('A focused desktop tool managed in PHP.')
    ->publisher('Example')
    ->category(ApplicationCategory::Productivity)
    ->icon('resources/icon.svg')
    ->excludeFromBundle('storage/cache', 'storage/development.sqlite');
```

Identifiers are lowercase reverse-DNS names of at most 155 bytes. Versions use
a portable numeric-leading format. Names, descriptions and publishers reject
control characters. Categories are transmitted as sequential integers:

| Value | Category | Freedesktop category |
| --- | --- | --- |
| `1` | Development | `Development` |
| `2` | Productivity | `Office` |
| `3` | Graphics | `Graphics` |
| `4` | AudioVideo | `AudioVideo` |
| `5` | Network | `Network` |
| `6` | Utility | `Utility` |
| `7` | Game | `Game` |
| `8` | Education | `Education` |

Icons must be project-relative PNG or SVG files below `resources`. PNG icons
must be square and between 64px and 1024px. SVG icons must be UTF-8,
self-contained and free of scripts, doctypes and remote resources.

Required application paths cannot be excluded. Pam also omits `.git`, `.pam`,
`.env*`, `dist`, `node_modules`, `target`, `.pamignore` and nested development
`vendor` directories from Composer path packages. Absolute `type=path`
repository locations are removed from the bundled Composer metadata after their
packages are materialized, so build-machine paths are not disclosed.

## Build formats

```bash
# directory + portable tar.gz
pam desktop build .

# one format
pam desktop build . --format directory
pam desktop build . --format portable
pam desktop build . --format deb

# all formats and an explicit output
pam desktop build . --output artifacts --format all
```

Existing destinations cause a failure. Pass `--force` only when those exact
versioned artifacts should be replaced.

The directory and portable builds contain:

```text
my-app-0.4.0-linux-x86_64/
├── app/                  PHP application and production Composer tree
├── bin/
│   ├── my-app            production launcher
│   ├── pam               embedded PHP worker runtime
│   └── pam-desktop       Servo host
├── etc/php.ini           isolated worker configuration
├── lib/                  non-glibc dynamic runtime libraries
├── share/
│   ├── applications/     Freedesktop template
│   └── icons/            validated application icon
├── install.sh            atomic per-user installer
├── uninstall.sh          scoped per-user removal
└── manifest.json         target metadata and file integrity
```

The per-user installer writes beneath `$XDG_DATA_HOME` (or
`$HOME/.local/share`) and links the executable beneath `$XDG_BIN_HOME` (or
`$HOME/.local/bin`). It updates the desktop database when the platform provides
that command.

Debian packages install the application under `/opt/<application-id>`, expose a
launcher under `/usr/bin`, and place desktop/icon metadata under `/usr/share`.
Creating them requires `dpkg-deb`; building portable formats does not.

## Reproducibility and integrity

Portable archives sort entries, normalize ownership to root and use
`SOURCE_DATE_EPOCH`. When that variable is absent, archive time is zero. The
bundle manifest records:

- schema and Pam Desktop protocol versions;
- the complete typed PHP application manifest;
- Pam Desktop and Pam runtime versions;
- operating system, architecture and ABI;
- every shipped relative path, byte count and SHA-256 digest.

The manifest deliberately excludes itself from the digest list. Verify a
bundle from its root with:

```bash
jq -r '.files[] | "\(.sha256)  \(.path)"' manifest.json | sha256sum -c -
```

Linux bundles retain a glibc and graphics-stack compatibility floor. Build on
the oldest supported distribution, currently Ubuntu 22.04-compatible, and test
the resulting artifact on every declared target distribution.

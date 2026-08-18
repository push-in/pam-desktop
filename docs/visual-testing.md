# Visual regression testing

PAM Desktop turns a captured project PNG into an exact, reproducible pixel
contract. Capture remains user-mediated: on Linux, enable the desktop portal and
call `pam.portal.screenshot()`; platform test drivers may instead write their
window capture directly under the project. The harness never reads an absolute
or parent-relative path.

Accept a reviewed baseline once:

```bash
pam desktop visual accept \
  --name settings.dark \
  --actual artifacts/screenshots/settings-dark.png
```

The golden is written to `tests/visual/settings.dark.png`. Existing goldens are
immutable unless `--force` is explicit. Commit the golden with the application.

Verify the same case locally or in CI:

```bash
pam desktop visual verify \
  --name settings.dark \
  --actual artifacts/screenshots/settings-dark.png
```

Verification decodes both PNGs to normalized RGBA pixels, so harmless PNG chunk
or compression differences do not fail a build. Dimensions and every pixel must
match. A mismatch exits unsuccessfully and reports the exact changed-pixel
count.

Each case overwrites one bounded evidence file at
`artifacts/visual/<case>.json`; repeated runs do not accumulate timestamped
captures or caches. Schema 1 records Desktop surface code `3`, comparison code
`1` for match or `2` for mismatch, dimensions, changed pixels, both source
digests, host OS/architecture, PAM Desktop version and the project Git revision
when available. CI should upload this directory together with its screenshots.

## Safety limits

- case names contain at most 64 lowercase letters, digits, dots or hyphens;
- source files must be project-relative `.png` files;
- compressed files are limited to 32 MiB;
- decoded images are limited to 64 MiB and 8,192 pixels per dimension;
- golden and evidence parents are resolved before writes, preventing symlink
  escapes;
- temporary writes are removed on both success and failure.

Use stable fonts, locale, theme, scale factor, dimensions and animation state in
the platform capture driver. Keep separate named cases when those variables are
intentional; do not overwrite one baseline with results from incompatible host
conditions.

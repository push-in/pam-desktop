# PAM Desktop gallery

The largest example: a gallery of four product-quality screens written with
`.pam` templates and plain components.

- a food marketplace with an interactive cart;
- a financial dashboard that works offline;
- a chat with a media composer;
- a field operations screen with sync state.

The engineering lab behind the gallery exercises persistent PHP state,
incremental rendering, HTTPS and storage, stack navigation, a remote image and
a virtualized list with 10,000 rows.

From the repository root:

```bash
composer install
cargo build -p pam-desktop-host
target/debug/pam-desktop dev examples/showcase
```

Templates are in `resources/views`, single-file components in
`resources/components` and the screens' classes in `src`.

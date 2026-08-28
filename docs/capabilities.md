# Native capabilities

Pam Desktop exposes operating-system features only after the PHP application
declares them. No capability is enabled by default, and the frontend never
receives a generic native bridge.

## Declare policy in PHP

```php
use Pam\Desktop\Capabilities;
use Pam\Desktop\Database;
use Pam\Desktop\FileSystemRoot;
use Pam\Desktop\HttpOrigin;
use Pam\Desktop\ProcessCommand;

$app->capabilities(
    Capabilities::none()
        ->filesystem(
            FileSystemRoot::read('assets', __DIR__.'/assets'),
            FileSystemRoot::readWrite('data', __DIR__.'/storage'),
        )
        ->dialogs()
        ->clipboard(read: true, write: true)
        ->notifications()
        ->dragAndDrop()
        ->database(Database::readWrite('app', 'storage/app.sqlite'))
        ->systemInformation()
        ->http(HttpOrigin::allow('api', 'https://api.example.com/v1'))
        ->secrets()
        ->process(ProcessCommand::executable('thumbnailer', 'bin/thumbnailer'))
        ->desktopPortal(),
);
```

Filesystem root paths may be project-relative or absolute because they are
trusted application policy, not browser input. The directory must already
exist. Root identifiers follow the same 64-byte identifier grammar as windows
and commands.

File access is transmitted as an integer:

| Value | PHP enum | Meaning |
| --- | --- | --- |
| `1` | `FileAccess::Read` | metadata, list and read text |
| `2` | `FileAccess::Write` | create directories and write text |
| `3` | `FileAccess::ReadWrite` | both permission sets |

## Filesystem

Named-root targets contain `root` and a relative `path`:

```js
const target = { root: "data", path: "notes/today.txt" };

await pam.fs.createDirectory({ root: "data", path: "notes" });
await pam.fs.writeText(target, "Olá, Pam.");
const text = await pam.fs.readText(target);
const metadata = await pam.fs.metadata(target);
const entries = await pam.fs.list({ root: "data", path: "notes" });
```

`readText` and `writeText` are limited to 8 MiB and accept UTF-8 text only.
Paths must be relative and cannot contain `..`. Symbolic links are never
returned or opened by the bridge. Directory entries contain `name`, relative
`path`, integer `kind` (`1` file, `2` directory), and byte `size`.

Every method except dialogs accepts an optional `{ timeout, signal }` argument
with the same client-side behavior as application commands.

Binary data uses `pam.fs.openRead()` and `pam.fs.writeStream()` instead of the
bounded JSON text methods. A named-root watcher uses `pam.fs.watch()` and emits
`pam.fs.changed`. See [Binary streaming](streaming.md) and
[File watching](file-watching.md).

## Dialogs and grants

```js
const file = await pam.dialog.openFile({
    title: "Open a document",
    filters: [
        { name: "Text", extensions: ["txt", "md"] },
    ],
    persistent: true,
});

const files = await pam.dialog.openFiles();
const destination = await pam.dialog.saveFile({ fileName: "report.txt" });
const directory = await pam.dialog.openDirectory({ access: 3 });
```

A cancelled single-selection dialog returns `null`; a cancelled multi-selection
dialog returns an empty array. Selected values have this shape:

```js
{
    grantId: "opaque-256-bit-token",
    name: "report.txt",
    kind: 1,
    access: 1
}
```

Pass the object directly to `pam.fs`, optionally adding a relative `path` for a
directory grant. Open-file grants are read-only, save-file grants are
read-write, and directory access defaults to read-only. Grants are ephemeral
unless the dialog sets `persistent: true`.

Dialogs run on the native Winit event loop. They intentionally do not accept a
timeout or `AbortSignal` because cancelling a fetch cannot safely dismiss an
operating-system picker on every platform.

Persistent grants retain at most 128 opaque IDs in a private atomic per-user
store. PAM revalidates the selected resource and symlink policy on every host
start and never exposes the absolute path to frontend code. Ephemeral grants
still expire on process exit or a successful PHP runtime hot reload.

## Clipboard

```js
await pam.clipboard.writeText("Copied by Pam Desktop");
const text = await pam.clipboard.readText();
await pam.clipboard.writeHtml("<strong>Rich text</strong>", "Rich text");

const image = await pam.clipboard.readImage();
await pam.clipboard.writeImage({
    width: image.width,
    height: image.height,
    rgbaBase64: image.rgbaBase64,
});
const files = await pam.clipboard.readFiles();
await pam.clipboard.writeFiles(files.map(({ grantId }) => ({
    root: null,
    grantId,
    path: "",
})));
await pam.clipboard.writeCustom(
    "application/vnd.example.selection+json",
    btoa(JSON.stringify({ documentId: 42, blocks: [1, 2] })),
);
const custom = await pam.clipboard.readCustom(
    "application/vnd.example.selection+json",
);
const formats = await pam.clipboard.availableFormats();
await pam.clipboard.clear();
```

PHP gates reads and writes independently. Text, HTML and decoded RGBA image
data are each limited to 1 MiB. HTML accepts a plain-text fallback. Image
dimensions are overflow-checked and must match the decoded RGBA length exactly.
File lists contain at most 256 regular files/directories. Reading returns
capability grants rather than ambient paths; writing accepts only readable
declared-root or grant targets and rejects symbolic-link traversal. The Rust
host retains one serialized clipboard instance for the application lifetime.
Custom payloads are real operating-system clipboard formats, not process-local
storage. Format names are restricted to printable `application/x-*` or
`application/vnd.*`, payloads use base64 and the same 1 MiB bound, and format
enumeration is capped at 256 entries.

## Notifications

```js
await pam.notification.show({
    title: "Export complete",
    body: "report.pdf is ready.",
    urgency: 2,
    actions: [
        { id: "open", label: "Open file" },
        { id: "folder", label: "Show in folder" },
    ],
});

pam.on("pam.notification.action", ({ notificationId, action }) => {
    console.log(notificationId, action);
});
```

Urgency is `1` low, `2` normal, or `3` critical. Titles are limited to 256
bytes and bodies to 4 KiB. A notification accepts up to four actions, each with
a unique bounded identifier and label. Linux publishes the selected action to
the originating window; platforms whose notification service does not expose
action callbacks still display the notification normally. Delivery and visual
presentation remain controlled by the desktop environment.

## Drag and drop

Subscribe through the existing event API:

```js
pam.on("pam.drag.enter", ({ name, kind }) => {
    console.log("hover", name, kind);
});

pam.on("pam.drag.leave", () => {
    console.log("left window");
});

pam.on("pam.drag.drop", async ({ files }) => {
    const [file] = files;
    console.log(await pam.fs.readText(file));
});

pam.on("pam.drag.error", ({ code, message }) => {
    console.error(code, message);
});
```

Dropped files and directories receive read-only grants targeted to the Winit
window under the cursor. Hover events expose only the display name and integer
kind; ambient paths never enter JavaScript.

## Specialized namespaces

SQLite databases and system inspection have their own capability gates and
frozen frontend namespaces. They do not inherit filesystem authority. The same
rule applies to named native HTTPS origins, Secret Service keys, allowlisted
processes and user-mediated XDG portals. See:

- [Native SQLite](database.md)
- [System information](system-information.md)
- [Native HTTP](http.md)
- [Linux secrets](secrets.md)
- [Authorized processes](processes.md)
- [Linux desktop portals](desktop-portals.md)

Lifecycle declarations are packaging/runtime policy rather than a renderer
capability; see [Linux lifecycle](lifecycle.md). Hardware and long-lived media
sessions belong in [process-isolated plugins](plugins.md).

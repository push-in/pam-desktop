# Migrating from 1.x

PAM Desktop 2.0 is not an upgrade of 1.x. It is a different product under the same name: 1.x showed a web interface in a Servo web view and let it call PHP through a typed command gateway; 2.0 renders a native interface that PHP describes with components.

**An application written for 1.x does not run on 2.0, and there is no compatibility layer.** Moving to 2.0 means writing the interface again. Application logic that does not touch the 1.x API (domain classes, database queries, HTTP clients) can be kept.

## What does not carry over

| 1.x | Why it is gone | In 2.0 |
| --- | --- | --- |
| The interface in HTML, CSS and JavaScript, rendered by Servo | 2.0 has no web view in the rendering path. | Elements and `Style` in PHP: [Components](components.md), [Styling](styling.md). A `WebView` element can still embed a page. |
| Typed commands (`#[Command]` methods called from JavaScript) and events emitted from PHP to JavaScript | There is no JavaScript side to call from or emit to. | Event handlers are PHP closures on elements; state is `#[State]` on components. |
| The capability gateway: capabilities off by default, named file roots, opaque grants, the permission audit | The gateway stood between web content and PHP. With no web content there is nothing for it to mediate. | The PHP worker is an ordinary PHP process with the rights of the user. The host modules have their own limits (a file sandbox, HTTPS only): [Modules](modules.md). |
| The application declared in PHP (`#[Desktop(...)]`, the protocol 6 manifest) | The host no longer boots PHP to learn what the application is. | `pam-desktop.json`: [Getting started](getting-started.md#pam-desktopjson). |
| Declarative windows and menus of 1.x | Replaced by the 2.0 shell API. | `UI\Window`, `App::menu()`, `Tray`, `Shortcut`: [Desktop shell](desktop-shell.md). |
| Rust plugins and their versioned contracts | The plugin host was part of the 1.x shell. | Not available. |
| Workstation features: multi-process window isolation, the background agent, workstation certification | They belonged to the 1.x host. | Not available. |
| The `pam desktop` commands of 1.x | They drove the 1.x host. | The 2.0 host has its own commands, run through `vendor/bin/pam-desktop` or directly: `pam-desktop init`, `dev` and `run`. Packaging is in [Distribution](distribution.md). |

A 1.x capability with no row above and no section in [Modules](modules.md) or [Desktop shell](desktop-shell.md) is not provided by 2.0.

Class names are not a guide. `Pam\Desktop\App`, for instance, exists in both versions: in 1.x it was a base class the application extended; in 2.0 it is a static entry point (`App::run()`, `App::menu()`).

## What stays

- **PHP runs in a worker process** started by a Rust host, as before. The host and the worker now exchange binary frames over the worker's standard input and output: [Architecture](architecture.md).
- **PHP 8.5.**
- **The Composer package name**, `pushinbr/pam-desktop`, and the `Pam\Desktop` namespace.
- **The host binary name**, `pam-desktop`, and the Composer launcher `vendor/bin/pam-desktop`. The launcher still reads `PAM_DESKTOP_HOST_BINARY`, the 1.x name of `PAM_DESKTOP_HOST`.
- **The repository and the license.**

## Where the 1.x code is

The 1.x sources are not part of the 2.0 tree. They are in the history of the repository, at the `v1.*` tags; the last one is `v1.3.4`.

The 1.x entries of the [changelog](../CHANGELOG.md) are unchanged.

## A way to start

1. Run `examples/counter` and read [Getting started](getting-started.md).
2. Rebuild one screen of your application with [Components](components.md), using the example closest to it under `examples/`.
3. Move persistence and network code to the [Modules](modules.md), or keep your own PHP code where it does not block: a module call returns at once and answers through a callback, while a blocking call in a handler delays the next render.
4. Rebuild menus, shortcuts and dialogs with the [Desktop shell](desktop-shell.md) API.

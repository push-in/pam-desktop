# Compatibility

The PHP package (`pushinbr/pam-desktop`) and the native host (`pam-desktop`) are released together and should be updated together. This page explains what happens when they are not, and which properties need a recent host.

## How the two sides agree

Before it starts the worker, the host exports `PAM_RENDERER_CONTRACT`: its ABI version, the protocol versions it speaks and the highest property and event identifiers it implements, for the core table and for the extension and desktop tables. The current host exports:

```json
{"abi":1,"events":[80,2007],"properties":[543,2028],"protocol":[1,1]}
```

At boot the PHP package negotiates with that contract:

- **Protocol.** Both sides speak protocol 1 (`PROTOCOL_VERSION` and `MINIMUM_PROTOCOL_VERSION` are 1). A host whose range does not include it stops the application before the first frame: `The installed Pam Desktop renderer is incompatible with this application…`.
- **Properties and events.** The package never sends an identifier above the ceiling the host declared. Rendering an element that needs one throws a `ProtocolMismatchException` naming it, for example `The installed Pam Desktop renderer does not implement property TextSpanBoxes (2028)… Rebuild the application with a renderer that ships this property.` In development it is shown by the error overlay.
- **No contract.** A host that exports nothing is treated as the core table with no extensions.

The host checks the same thing from its side: a frame with a property id above its own table is a protocol error (`UnknownProperty`).

So an older host keeps running an application as long as the screen on view uses nothing it lacks; the first screen that does fails with a message naming the property.

## Desktop identifiers by feature

The desktop table starts at 2001. These are the identifiers added after the first 2.0 tree, with the feature that sends them. A host must declare at least the id in the first column.

| Property | Id | Sent by |
| --- | --- | --- |
| `CaptureKeys` | 2017 | `captureKeys` |
| `DragType`, `DropType` | 2018, 2019 | `DragSource::payload()`, `DropTarget::accepts()`, `dragType`, `dropType` |
| `OnDragEnter`, `OnDragLeave` | 2020, 2021 | `DropTarget::onDragEnter()`, `onDragLeave()` |
| `GridTracks` | 2022 | `grid-template-columns` with `fr` tracks, `Grid::tracks()` |
| `InputFormatDecimalMinDigits` | 2023 | `Input::decimal()`, `inputFormat="decimal"` |
| `ScrollbarGutter` | 2024 | `scrollbar-gutter` |
| `ScrollbarWidth` | 2025 | `scrollbar-width: auto \| thin \| none` |
| `ScrollbarThickness` | 2026 | `scrollbar-width: <length>` |
| `ScrollbarColor` | 2027 | `scrollbar-color` |
| `TextSpanBoxes` | 2028 | A nested `Text` with padding, a border, or a background and a radius |

| Event | Id | Reported to |
| --- | --- | --- |
| `DragEnter`, `DragLeave` | 2006, 2007 | `onDragEnter`, `onDragLeave` |

The full desktop table, with the identifiers from 2001, is in [Architecture](architecture.md#property-and-event-tables).

## Native modules

A module call to a host that does not have the module (`websocket`, `printing`, `files.save`, `sqlite.transactionIf` are recent) fails through the failure callback of the call, like any other module error; see [Modules](modules.md#failures).

## In practice

- `vendor/bin/pam-desktop` downloads the host of the installed package version, so the two match.
- `PAM_DESKTOP_HOST` points at a host built from source: rebuild it when you update the package from a checkout.
- A bundle made by `pam-desktop build` ships the host it was built with.

## Related

- [Architecture](architecture.md) for the frame protocol.
- [Development](development.md) for the error overlay.

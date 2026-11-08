# Performance

What a PAM Desktop application costs on Linux, measured against an Electron
application that does the same job, and what the framework does by default
to keep that cost low. Nothing here needs configuration.

## Measured against Electron

A point-of-sale application written with PAM Desktop and its Electron
counterpart (Electron 37, production build), both with the same 2500-product
local catalog, in the same disposable container: Ubuntu 24.04, Xvfb
1200x800, 4 CPUs, no network, pt-BR. Both draw in software there (no GPU:
GTK's Cairo renderer, Chromium's SwiftShader). Medians; cold starts are 10
runs in interleaved blocks after one discarded warm-up, so the operating
system cache is warm for both.

| | PAM Desktop | Electron | PAM / Electron |
| --- | ---: | ---: | ---: |
| Cold start, login screen complete | 138 ms | 1563 ms | 9% |
| Cold start, signed-in dashboard complete | 151 ms | 1469 ms | 10% |
| Cold start until the checkout screen (start + one click) | 281 ms | 2641 ms | 11% |
| Route change (median of 17 screens) | 41 ms | 322 ms | 13% |
| Key press to visible change, median / p95 | 12.8 / 20.5 ms | 17.6 / 122 ms | |
| Memory idle (PSS, every process) | 75 MB | 322 MB | 23% |
| Memory after 18 screens | 135 MB | 606 MB (363 MB 20 s later) | 22% (37%) |
| Memory after scrolling 2500 products | 143 MB | 657 MB | 22% |
| CPU idle, visible / minimized (% of one core) | 0.2 / 0.0 | 1.9 / 1.0 | |
| Scrolling 2500 products down: updates per second, gaps over 33 / 50 ms | 55–58, 1–5 / 0 | 59, 0 | |
| Scrolling them back up: updates per second | 53–54 | 60 | |
| CPU of the host while scrolling down / up | 1.0 / 0.65 s | | |
| Package: directory / `.tar.gz` | 120 / 42 MB | 468 / 144 MB | 26% / 29% |

PAM Desktop runs two processes (the host and the PHP worker); Electron six.
The second round (memory and scrolling rows) was measured with the same
method, three interleaved runs of the framework before and after the
changes below, against the Electron figures of the first round:

| | Before | After |
| --- | ---: | ---: |
| Memory idle: host / worker / total | 39 / 43 / 82 MB | 34 / 40 / 75 MB |
| Memory after 18 screens: host / worker / total | 78 / 86 / 164 MB | 63 / 72 / 135 MB |
| Scrolling down: updates per second, longest gap, gaps over 50 ms | 52.4, 61 ms, 1 | 55.4 (55–58), 45 ms, 0 |
| Scrolling up: updates per second | 49.5 | 53.5 |
| Host CPU while scrolling down / up | 1.56 / 1.68 s | 0.99 / 0.65 s |
| Cold start, dashboard complete | 180 ms | 175 ms |

Scrolling is where Electron still leads in software rendering. The frames
of the scroll itself now cost 2 to 6 ms and come at a steady 16.7 ms; what
still drops a frame is the work of rows joining the list on the UI thread:
the worker sends rows in windows of 64 items, and applying one (layout of
about 1000 new nodes and their GTK widgets) takes 10 to 20 ms. Chromium
builds and rasterizes off its compositor thread. With a GPU both are
hardware composited.

## What the framework does

- **Compiled PHP is kept between launches.** The host starts the worker with
  OPcache's file cache in the application's cache directory
  (`opcache.file_cache_only`, no shared memory segment): from the second
  launch on, neither the application nor the framework is compiled again.
  Cold start to the first complete screen went from 280–336 ms to 151 ms.
  `pam exec` (and a bundle's PHP) receives the settings through
  `PAM_INI_ENTRIES`, a plain `php` through `-d`; settings of your own in
  `PAM_INI_ENTRIES` still apply after them.
- **No OpenGL probe without a GPU.** On a machine without a DRM device or
  the NVIDIA device (a virtual machine, a container, a remote or headless
  session) GTK loaded the system OpenGL stack only to end up drawing with
  Cairo: Mesa's llvmpipe kept about 60 MB in the host. The host now tells GTK
  not to probe there (`GDK_DEBUG=gl-disable`, `GDK_DISABLE=gl`). The host went
  from 93–110 MB to about 40 MB. A GL setting of your own (`GDK_DISABLE`,
  `GDK_DEBUG=gl-…`) or a GPU renderer in `GSK_RENDERER` is kept.
- **Components are precompiled into the bundle.** `pam-desktop build`
  compiles every `.pam.php` component of the bundled application into its
  prebuilt pack (`pam-prebuilt/components`), so the installed application
  does not compile them at its first launch.
- **Rows scroll from images in software rendering.** Under GTK's Cairo
  renderer (no GPU) GTK kept every widget's render node but drew the text,
  borders and clips of the visible rows again at each frame of a scroll.
  While a scroll view scrolls, each child of its content on screen is
  rasterized once into an image placed on whole device pixels, rows off
  screen are left out of the frame, and a row is drawn again only when its
  render node changes (one that keeps changing is drawn directly). A second
  after the scroll ends the images are released. GPU renderers never use it.
- **A steady frame cadence while scrolling.** Without a compositor that
  reports presented frames, GTK timed each frame a scroll asked for from the
  late start of the previous one: 18 to 19 ms instead of 16.7. The frame
  clock is kept in its animation mode while a scroll view moves and for
  150 ms after.
- **Worker frames are applied after the frame being drawn.** A backend event
  (a scroll offset GTK changes during layout, a wheel step) drains only the
  backend's events; rows and module results from PHP are applied by the
  worker's own pump at idle priority, after the frame.
- **Rows joining a list are laid out incrementally.** A structural patch lays
  out the nodes it touched and their ancestors instead of the whole tree;
  debug builds compare every such pass with a full layout.
- **Freed memory goes back to the system.** Half a second after the windows
  stop painting, when more than a megabyte of heap is free, the GTK host
  calls `malloc_trim` (glibc keeps what a frame freed otherwise).
- **Compiler-only style data is not kept.** The style IR, its bytecode,
  source map and compatibility report of each component's stylesheet were
  decoded and kept by the worker for every component shown, although
  rendering never reads them: about 12 MB after 18 screens of the
  point-of-sale application.
- **Prebuilt components are compiled for the mode the bundle runs in.**
  `pam-desktop build` compiled them in development mode and an installed
  application runs in production (strict) mode, so the pack was never used
  and every component was compiled again at the first launch.
- **A read-only installation boots.** When the component cache an
  application names next to itself cannot be written (a system package, an
  AppImage), components go to a private cache below the per-user cache
  directory instead of stopping the boot.

## Tried and left out

- Smaller row windows (margins of 24 or 32 items, two margins kept instead
  of four): more, not cheaper, commits; scrolling did not improve.
- Measuring each node's intrinsic size once per layout pass: no gain.
- `opcache.save_comments=0`, and `gc_collect_cycles()` with
  `gc_mem_caches()` once the worker is idle: no change in the worker's
  memory (what it holds is live; OPcache runs from its file cache, without a
  shared memory segment, so `opcache.memory_consumption`,
  `interned_strings_buffer` and `jit_buffer_size` allocate nothing).

## Bundles

- Directories tagged as caches with a `CACHEDIR.TAG` (Cargo targets under
  any name, for instance) stay out of the bundle; a stray Cargo target had
  made one bundle 400 MB.
- With the PAM runtime, the build asks PHP through the runtime's own
  launcher, from a neutral directory, for its version and shared extensions,
  ships the ones the application uses (and the libraries of the runtime's
  `lib` directory they link against), and ships the runtime binary, not its
  launcher script. Before, such a bundle had no extension at all and stopped
  at boot.

The PHP runtime is the largest part of a bundle (98 of 120 MB): `intl` brings
ICU's data (29 MB) and `gd` its image codecs. They are shipped only when the
application refers to them; `"extensions"` in `pam-desktop.json` lists what
the search cannot see.

## Reproducing

The measurements use a frame probe that polls the virtual display with
`XGetImage` every 2 ms and sends input with XTest (150 wheel steps 16 ms
apart over the product grid), and sum `/proc/*/smaps_rollup` and
`/proc/*/stat` over each application's process tree. Memory is read 3 s
after the dashboard settles, and 1 s and 20 s after a walk of 18 screens. Run both applications in the same container, with the same data,
interleaving them so that the machine's load affects both alike.

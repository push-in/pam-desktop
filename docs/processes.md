# Authorized processes

Pam Desktop does not expose `exec`, a shell, or an arbitrary program path to
JavaScript. Applications name individual bundled executables in PHP:

```php
use Pam\Desktop\ProcessCommand;

$app->capabilities(
    Capabilities::none()->process(
        ProcessCommand::executable('thumbnailer', 'bin/thumbnailer')
            ->arguments('--format', 'webp')
            ->allowArguments(),
    ),
);
```

```js
const result = await window.pam.process.run("thumbnailer", {
  arguments: ["input.png"],
  stdin: "",
  timeout: 10_000,
});
```

Argument policy is an integer enum: `1` fixed arguments only and `2` append
bounded frontend arguments. The host rejects paths outside the project,
symlinks, non-executable files, more than 32 arguments, NUL bytes, timeouts
outside 100–120,000 ms, stdin over 1 MiB, and stdout/stderr over 1 MiB. It never
passes input through a shell, clears the inherited environment, captures output
concurrently to avoid pipe deadlocks, kills timed-out children and returns
`success`, `exitCode`, `stdout` and `stderr`.

Use process plugins for persistent or richer native integrations. This API is
for small, explicit tools whose authority is reviewable in the manifest.

## Interactive PTY sessions

The same allowlist can open a real platform PTY for terminals, REPLs and language servers. No arbitrary shell path is accepted.

```js
const session = await pam.terminal.open("project-shell", {
  arguments: ["--noprofile"],
  columns: 120,
  rows: 36,
});

await pam.terminal.write(session.sessionId, "php -v\r");
const chunk = await pam.terminal.read(session.sessionId);
terminal.write(chunk.bytes); // Uint8Array; preserves ANSI and UTF-8 boundaries

await pam.terminal.resize(session.sessionId, 160, 48);
await pam.terminal.close(session.sessionId);
```

Operations use sequential integer codes: run `1`, open `2`, write `3`, read `4`, resize `5`, close `6`. Output is transported as bounded binary chunks instead of being accumulated in memory. Resize reaches the operating-system PTY, which notifies the child; close terminates the process and releases all handles. A session reports `running`, `exitCode` and the terminating signal.

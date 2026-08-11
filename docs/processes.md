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

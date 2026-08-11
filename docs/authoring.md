# Convention-first authoring

PAM Desktop offers one runtime with two authoring levels. The convention-first
API is the default for applications; the immutable builders remain available
for infrastructure, plugins, generators, and unusual host policy.

## The smallest useful application

```php
<?php

declare(strict_types=1);

namespace App;

use Pam\Desktop\App;
use Pam\Desktop\Attributes\Command;
use Pam\Desktop\Attributes\Desktop;
use Pam\Desktop\WindowHandle;

#[Desktop(id: 'com.example.hello', name: 'Hello')]
final class HelloApp extends App
{
    #[Command]
    public function greet(string $name = 'world', WindowHandle $window): array
    {
        $window->title("Hello, {$name}");

        return ['message' => "Hello, {$name}!"];
    }
}
```

`resources/index.html` is the default page. The default window is 1120 × 720,
has a 720 × 520 minimum, follows the system theme, and receives no native
permissions. The entry point calls `HelloApp::run()`.

## What `#[Command]` does

For an invocation such as:

```js
await window.pam.invoke("documents.save", {
  document: { id: 42, title: "Roadmap" },
  notify: true,
});
```

PAM can invoke a normal typed method:

```php
#[Command('documents.save')]
public function save(
    DocumentData $document,
    bool $notify,
    DocumentRepository $documents,
): DocumentResource {
    $saved = $documents->save($document);

    return new DocumentResource($saved);
}
```

Scalar fields bind by parameter name. A nested JSON object hydrates a typed DTO
through its constructor. Integer-backed enums are created through `from()`.
Class dependencies resolve through the container. Missing values use PHP
defaults or nullable types; incompatible input fails with a bounded handler
error instead of coercing silently.

Arrays, scalars, objects, `null`, and `CommandResult` are valid returns. PAM
normalizes ordinary values to a successful command response.

## Constructor injection and bindings

Concrete classes are autowired recursively:

```php
final class Documents extends App
{
    public function __construct(
        private readonly DocumentRepository $documents,
    ) {}
}
```

Interfaces and custom instances use bindings:

```php
protected static function bindings(): array
{
    return [
        Clock::class => new SystemClock(),
        DocumentRepository::class => new SqliteDocumentRepository(),
    ];
}
```

Bindings are explicit and application-local. The container never scans global
state or silently constructs scalar configuration.

## Permissions remain visible

Security-sensitive access is never inferred from source code:

```php
protected function configure(Desktop $desktop): void
{
    $desktop->permissions(static fn (Permissions $permissions) => $permissions
        ->filesystem('data', __DIR__.'/../storage', read: true, write: true)
        ->database('app', 'storage/app.sqlite')
        ->http('api', 'https://api.example.com/v1')
        ->dialogs()
        ->clipboard()
        ->notifications()
    );
}
```

This concise declaration produces the same capability manifest as the advanced
`Capabilities` builder. Rust remains the enforcement boundary.

## Advanced API

The convention layer is additive. Existing applications may continue to use:

```php
$application = Application::make(...)
    ->capabilities(Capabilities::none()->dialogs())
    ->shell(Shell::none());
```

Both styles produce protocol 6 boot data and use the same deadlines, worker
recovery, native gateway, capability checks, plugins, packaging, and updater.

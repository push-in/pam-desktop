# Typed commands, DTOs, events, and dependency injection

## Method commands

`#[Command]` without a name converts camel case to dotted lower case:

```php
#[Command]
public function openSettings(): void {}
```

The frontend command is `open.settings`. Use an explicit identifier when an API
contract must not follow the PHP method name:

```php
#[Command('settings.open', execution: CommandExecution::Parallel)]
public function openSettings(): void {}
```

Execution modes remain integer-backed in protocol 6.

## Command classes

Large applications can keep one use case per class:

```php
#[Command('documents.save')]
final class SaveDocument
{
    public function __construct(private DocumentRepository $documents) {}

    public function __invoke(DocumentData $document): DocumentResource
    {
        return new DocumentResource($this->documents->save($document));
    }
}
```

Register command classes from the application:

```php
protected function commands(): array
{
    return [SaveDocument::class, DeleteDocument::class];
}
```

## Contextual services

The following services are scoped to one invocation:

| Service | Purpose |
| --- | --- |
| `CommandContext` | Request identity, command name, source window, raw payload |
| `WindowHandle` | Current-window effects |
| `Windows` | Validated lookup for any declared window |
| typed `DesktopWindow` subclass | A specific named window |
| `Events` | PHP-to-frontend events |
| `Invocation` | Low-level effect/event collector |

They must not be retained after the handler returns.

## Event objects

Any public readonly object can be emitted:

```php
final readonly class DocumentSaved
{
    public function __construct(
        public int $documentId,
        public int $revision,
    ) {}
}

$events->emit(new DocumentSaved($document->id, $document->revision));
```

`DocumentSaved` becomes `document.saved`; public properties become the payload.
Implement `Event` when the wire name or payload needs an explicit stable shape.

Frontend-to-PHP events use `#[Listen]` and the same typed binding rules:

```php
#[Listen('editor.changed')]
public function editorChanged(string $documentId, WindowHandle $window): void
{
    $window->title("Editing {$documentId}");
}
```

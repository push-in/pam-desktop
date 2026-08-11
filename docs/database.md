# Native SQLite

Pam Desktop exposes SQLite as a capability-scoped native service. The database
engine is compiled into the Rust host, so production bundles do not depend on a
system SQLite installation and do not ship a JavaScript database runtime.

## Declare databases in PHP

```php
use Pam\Desktop\Capabilities;
use Pam\Desktop\Database;

$app->capabilities(
    Capabilities::none()
        ->database(Database::readWrite('app', 'storage/app.sqlite'))
        ->database(Database::read('catalog', 'resources/catalog.sqlite')),
);
```

Database names use the standard PAM identifier grammar. Paths are always
project-relative, cannot contain parent components and cannot be symbolic
links. A read-write database creates its parent directory when necessary. A
read-only database must already exist.

## Query and mutate

```js
await pam.database.execute(
    "app",
    "CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
);

const inserted = await pam.database.execute(
    "app",
    "INSERT INTO notes (body) VALUES (?)",
    ["Ship it"],
);

const notes = await pam.database.query(
    "app",
    "SELECT id, body FROM notes ORDER BY id DESC LIMIT ?",
    [50],
);
```

Parameters may be null, booleans, signed 64-bit integers, finite numbers or
strings. Objects and arrays are rejected. Binary columns are deliberately not
returned through the JSON query API; use an authorized file or streaming
plugin for large binary data.

## Transactions

```js
await pam.database.transaction("app", [
    { sql: "UPDATE accounts SET balance = balance - ? WHERE id = ?", parameters: [50, 1] },
    { sql: "UPDATE accounts SET balance = balance + ? WHERE id = ?", parameters: [50, 2] },
]);
```

Every statement succeeds and the transaction commits, or SQLite rolls the
complete operation back. Read-write databases enable WAL, foreign keys and a
five-second busy timeout.

## Limits and security

| Resource | Limit |
| --- | ---: |
| SQL statement | 64 KiB |
| Parameters per statement | 1,024 |
| Rows per query | 10,000 |
| Columns per query | 256 |
| Statements per transaction | 256 |

The frontend never selects an arbitrary database path. Hot reload prepares the
entire replacement service before swapping it into the gateway, so a broken
database configuration does not partially replace the running capability set.

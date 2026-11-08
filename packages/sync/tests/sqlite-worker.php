<?php

declare(strict_types=1);

// Test process only. Bind the same scalar types as Rust/rusqlite: PDO's
// execute(array) binds integers as text and can change SQLite expression semantics.
function executeStatement(SQLite3 $connection, string $sql, array $arguments, bool $query): array
{
    $statement = $connection->prepare($sql);
    foreach (array_values($arguments) as $index => $value) {
        $type = match (true) {
            $value === null => SQLITE3_NULL,
            is_bool($value), is_int($value) => SQLITE3_INTEGER,
            is_float($value) => SQLITE3_FLOAT,
            is_string($value) => SQLITE3_TEXT,
            default => throw new RuntimeException('SQLite arguments must be JSON scalars.'),
        };
        $statement->bindValue($index + 1, is_bool($value) ? (int) $value : $value, $type);
    }
    $result = $statement->execute();
    $rows = [];
    if ($query) {
        while (($row = $result->fetchArray(SQLITE3_ASSOC)) !== false) {
            if (count($rows) >= 1000 || count($row) > 256) throw new RuntimeException('SQLite query exceeds native row/column limits.');
            $rows[] = $row;
        }
        if (strlen(json_encode($rows, JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES)) > 1_048_512) throw new RuntimeException('SQLite query exceeds native byte limit.');
    }
    $result->finalize();
    $statement->close();
    return $rows;
}

$connections = [];
while (($line = fgets(STDIN)) !== false) {
    $inTransaction = false;
    try {
        $request = json_decode($line, true, flags: JSON_THROW_ON_ERROR);
        $name = $request['database'];
        if (preg_match('/^[A-Za-z0-9_.-]{1,128}$/D', $name) !== 1) throw new RuntimeException('Invalid test database.');
        if (!isset($connections[$name])) {
            $connection = new SQLite3($argv[1].'/'.$name);
            $connection->enableExceptions(true);
            $connection->busyTimeout(5000);
            $connection->exec('PRAGMA journal_mode = WAL');
            $connection->exec('PRAGMA foreign_keys = ON');
            $connections[$name] = $connection;
        }
        $connection = $connections[$name];
        $arguments = json_decode($request['arguments'], true, flags: JSON_THROW_ON_ERROR);
        $applied = null;
        if ($request['method'] === 'transactionIf') {
            $connection->exec('BEGIN IMMEDIATE');
            $inTransaction = true;
            $guard = $connection->prepare($request['sql']);
            if (!$guard->readOnly()) throw new RuntimeException('Guard must be readonly.');
            $guard->close();
            $guardRows = executeStatement($connection, $request['sql'], $arguments['guardArguments'], true);
            if (count($guardRows) !== 1 || count($guardRows[0]) !== 1 || !in_array(array_values($guardRows[0])[0], [0, 1], true)) throw new RuntimeException('Guard must return one integer boolean.');
            $applied = (string) array_values($guardRows[0])[0];
            if ($applied === '1') foreach ($arguments['statements'] as $step) foreach ($step['argumentSets'] ?? [$step['arguments'] ?? []] as $set) executeStatement($connection, $step['sql'], $set, false);
            $connection->exec('COMMIT');
            $inTransaction = false;
            $rows = [];
        } elseif ($request['method'] === 'transaction') {
            $connection->exec('BEGIN');
            $inTransaction = true;
            foreach ($arguments as $step) {
                foreach ($step['argumentSets'] ?? [$step['arguments'] ?? []] as $set) {
                    executeStatement($connection, $step['sql'], $set, false);
                }
            }
            $connection->exec('COMMIT');
            $inTransaction = false;
            $rows = [];
        } else {
            $rows = executeStatement($connection, $request['sql'], $arguments, $request['method'] === 'query');
        }
        echo json_encode(['rows' => $rows, 'applied' => $applied], JSON_THROW_ON_ERROR)."\n";
    } catch (Throwable $error) {
        if ($inTransaction) $connection->exec('ROLLBACK');
        echo json_encode(['error' => $error->getMessage()], JSON_THROW_ON_ERROR)."\n";
    }
}

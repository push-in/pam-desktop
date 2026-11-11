<?php

declare(strict_types=1);

namespace Pam\Desktop\Database;

use Closure;
use InvalidArgumentException;

/**
 * Versioned schema changes for a SQLite database, applied once each.
 *
 * The version lives in the database itself (`PRAGMA user_version`), so a
 * fresh install runs every migration and an update runs only the new ones.
 * All pending migrations and the new version are written in one
 * transaction: a failure leaves the database as it was.
 *
 * ```php
 * Migrations::run('app.db', [
 *     1 => 'CREATE TABLE boards (id TEXT PRIMARY KEY, title TEXT NOT NULL)',
 *     2 => [
 *         'CREATE TABLE cards (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id), title TEXT)',
 *         'CREATE INDEX cards_board ON cards (board_id)',
 *     ],
 *     3 => 'ALTER TABLE cards ADD COLUMN due_at INTEGER',
 * ], fn (int $version, array $applied) => $this->load());
 * ```
 */
final class Migrations
{
    private function __construct()
    {
    }

    /**
     * @param array<int, string|list<string>> $migrations version (1 or more) => one statement or a list of them
     * @param (Closure(int, list<int>): void)|null $done the version the database is at, and the versions applied now
     * @param (Closure(\Pam\Desktop\Modules\NativeModuleException): void)|null $onError
     */
    public static function run(string $database, array $migrations, ?Closure $done = null, ?Closure $onError = null): void
    {
        $migrations = self::validated($migrations);
        SQLite::query($database, 'PRAGMA user_version', [], static function (array $rows) use ($database, $migrations, $done, $onError): void {
            $current = (int) ($rows[0]['user_version'] ?? 0);
            $pending = array_filter($migrations, static fn (int $version): bool => $version > $current, ARRAY_FILTER_USE_KEY);
            if ($pending === []) {
                $done?->__invoke($current, []);

                return;
            }
            $target = (int) array_key_last($pending);
            $statements = [];
            foreach ($pending as $sql) {
                foreach ($sql as $statement) {
                    $statements[] = ['sql' => $statement];
                }
            }
            $statements[] = ['sql' => "PRAGMA user_version = {$target}"];
            // The guard checks the version did not move since it was read
            // (another window migrating at the same time).
            SQLite::transactionIf(
                $database,
                'SELECT user_version = ? FROM pragma_user_version',
                [$current],
                $statements,
                static function (bool $applied) use ($database, $migrations, $done, $onError, $target, $pending): void {
                    if (!$applied) {
                        self::run($database, $migrations, $done, $onError);

                        return;
                    }
                    $done?->__invoke($target, array_keys($pending));
                },
                $onError,
            );
        }, $onError);
    }

    /**
     * @param array<int, string|list<string>> $migrations
     * @return array<int, list<string>> sorted by version
     */
    private static function validated(array $migrations): array
    {
        if ($migrations === []) {
            throw new InvalidArgumentException('Migrations needs at least one migration.');
        }
        $sorted = [];
        foreach ($migrations as $version => $sql) {
            if (!is_int($version) || $version < 1 || $version > 2_147_483_647) {
                throw new InvalidArgumentException('Migration versions are integers from 1.');
            }
            $statements = is_array($sql) ? array_values($sql) : [$sql];
            if ($statements === []) {
                throw new InvalidArgumentException("Migration {$version} has no statements.");
            }
            foreach ($statements as $statement) {
                if (!is_string($statement) || trim($statement) === '') {
                    throw new InvalidArgumentException("Migration {$version} must be SQL strings.");
                }
            }
            $sorted[$version] = $statements;
        }
        ksort($sorted);

        return $sorted;
    }
}

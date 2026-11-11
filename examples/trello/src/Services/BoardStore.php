<?php

declare(strict_types=1);

namespace App\Services;

use Closure;
use Pam\Desktop\Database\Migrations;
use Pam\Desktop\Database\SQLite;

/**
 * Keeps the board between launches in SQLite: where every card is and its
 * title and state. The schema evolves through migrations.
 */
final class BoardStore
{
    public const string DATABASE = 'board.db';

    private const array MIGRATIONS = [
        1 => 'CREATE TABLE cards (card_id TEXT PRIMARY KEY, list_id TEXT NOT NULL, position INTEGER NOT NULL, title TEXT NOT NULL, complete INTEGER NOT NULL DEFAULT 0)',
        2 => [
            'ALTER TABLE cards ADD COLUMN watching INTEGER NOT NULL DEFAULT 0',
            'CREATE INDEX cards_list ON cards (list_id, position)',
        ],
    ];

    /** Brings the schema up to date, then puts the saved board into `$board`. */
    public static function open(Board $board, Closure $ready): void
    {
        Migrations::run(self::DATABASE, self::MIGRATIONS, static function () use ($board, $ready): void {
            SQLite::query(self::DATABASE, 'SELECT card_id, list_id, position, title, complete, watching FROM cards ORDER BY list_id, position', [], static function (array $rows) use ($board, $ready): void {
                $board->restore($rows);
                $ready();
            });
        });
    }

    public static function save(Board $board): void
    {
        SQLite::transaction(self::DATABASE, [
            ['sql' => 'DELETE FROM cards'],
            ['sql' => 'INSERT INTO cards (card_id, list_id, position, title, complete, watching) VALUES (?, ?, ?, ?, ?, ?)', 'argumentSets' => $board->snapshot()],
        ]);
    }
}

<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

use Pam\Desktop\Database\Migrations;

// The SQL itself runs in the host (checked end to end against SQLite);
// here, the list of migrations is checked before anything is sent.
foreach ([
    'empty list' => [],
    'version 0' => [0 => 'CREATE TABLE a (id)'],
    'string version' => ['one' => 'CREATE TABLE a (id)'],
    'no statements' => [1 => []],
    'blank statement' => [1 => ['  ']],
    'non-string statement' => [1 => [42]],
] as $case => $migrations) {
    try {
        Migrations::run('app.db', $migrations);
        throw new RuntimeException("Migrations accepted an invalid list ({$case}).");
    } catch (InvalidArgumentException) {
    }
}
echo "Migrations validation passed\n";

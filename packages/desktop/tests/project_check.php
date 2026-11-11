<?php

declare(strict_types=1);

// `pam-desktop check` compiles and validates every component of an app in a
// separate PHP process, without the native host, and lists every diagnostic.
$assert ??= static function (bool $condition, string $message): void {
    if (!$condition) {
        fwrite(STDERR, $message."\n");
        exit(1);
    }
};

// The launcher as Composer's bin proxy runs it, with an autoloader of the
// SDK sources, so the suite needs no `composer install` of the fixtures.
$checkTools = sys_get_temp_dir().'/pam-desktop-project-check-'.getmypid();
@mkdir($checkTools, 0o700, true);
$source = var_export(realpath(__DIR__.'/../src'), true);
file_put_contents($checkTools.'/autoload.php', <<<PHP
    <?php
    spl_autoload_register(static function (string \$class): void {
        if (str_starts_with(\$class, 'Pam\\\\Desktop\\\\')) {
            \$path = {$source}.'/'.str_replace('\\\\', '/', substr(\$class, 12)).'.php';
            if (is_file(\$path)) require \$path;
        }
    });
    foreach (['Polyfill/mbstring_functions.php', 'Routing/functions.php', 'functions.php', 'Testing/functions.php'] as \$file) {
        require_once {$source}.'/'.\$file;
    }
    PHP);
file_put_contents($checkTools.'/pam-desktop', '<?php $GLOBALS[\'_composer_autoload_path\'] = '
    .var_export($checkTools.'/autoload.php', true).'; require '
    .var_export(realpath(__DIR__.'/../bin/pam-desktop'), true).';');
register_shutdown_function(static function () use ($checkTools): void {
    @unlink($checkTools.'/autoload.php');
    @unlink($checkTools.'/pam-desktop');
    @rmdir($checkTools);
});

$projectCheck = static function (string $project, string ...$arguments) use ($checkTools): array {
    $process = proc_open(
        [PHP_BINARY, $checkTools.'/pam-desktop', 'check', ...$arguments],
        [1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
        $pipes,
        $project,
        // A host override must not be started: check never reaches it.
        [...getenv(), 'PAM_DESKTOP_HOST' => '/nonexistent/pam-desktop'],
    );
    $output = stream_get_contents($pipes[1]);
    $errors = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);

    return [proc_close($process), (string) $output, (string) $errors];
};

$fixtures = __DIR__.'/Fixtures/ProjectCheck';

[$status, $output, $errors] = $projectCheck($fixtures.'/broken');
$expected = [
    'src/Screens/Badge.pam.php:21:1 PAM2403 Expected a native numeric CSS value in src/Screens/Badge.pam.php, got banana',
    'src/Screens/Header.pam.php:1:1 PAM2402 Unexpected closing tag Row in src/Screens/Header.pam.php',
    'src/Screens/Profile.pam.php:17:9 PAM2401 unknown attribute Text.color, did you mean textColor?',
    'src/Screens/Profile.pam.php:18:9 PAM2401 action save does not exist on Pam\Desktop\Tests\ProjectCheck\Broken\Screens\Profile',
    'src/Screens/Profile.pam.php:18:36 PAM2401 unknown attribute Text.maxLines, did you mean numberOfLines?',
    'src/Screens/Profile.pam.php:19:9 PAM2401 class ghost has no style rule in this component, no registered class and no utility',
    'src/Screens/Profile.pam.php:20:9 PAM2401 class missing has no style rule in this component, no registered class and no utility',
    'src/Screens/Totals.pam.php:22:9 PAM2405 template expression: ended unexpectedly in $count +',
    'src/Screens/Totals.pam.php:23:9 PAM2405 template expression: method missing() does not exist on Pam\\Desktop\\Tests\\ProjectCheck\\Broken\\Screens\\Totals',
    'pam-desktop check: 9 diagnostics (5 components in 1 root).',
];
$assert(
    $status === 1 && explode("\n", trim($output)) === $expected,
    "pam-desktop check must list every diagnostic of the app with file:line:col and exit 1:\n{$output}{$errors}",
);
$assert(
    !is_dir($fixtures.'/broken/.pam-desktop'),
    'pam-desktop check must compile into a private cache, not the app cache.',
);

[$status, $output] = $projectCheck($fixtures.'/broken', '--json');
$report = json_decode($output, true);
$assert(
    $status === 1
        && ($report['ok'] ?? null) === false
        && ($report['components'] ?? null) === 5
        && count($report['diagnostics'] ?? []) === 9
        && ($report['diagnostics'][2] ?? null) === [
            'file' => 'src/Screens/Profile.pam.php',
            'line' => 17,
            'column' => 9,
            'code' => 'PAM2401',
            'message' => 'unknown attribute Text.color, did you mean textColor?',
        ],
    "pam-desktop check --json must report the diagnostics as data:\n{$output}",
);

[$status, $output, $errors] = $projectCheck($fixtures.'/clean');
$assert(
    $status === 0 && trim($output) === 'pam-desktop check: 1 component in 1 root, no diagnostics.',
    "pam-desktop check must exit 0 on a clean app:\n{$output}{$errors}",
);

// --src checks a directory instead of the entry's roots; routes.php is checked too.
[$status, $output, $errors] = $projectCheck(__DIR__.'/Fixtures/RoutesApp', '--src=bad');
$assert(
    $status === 1
        && str_contains($output, "bad/Broken.php:15:1 PAM2404 unknown route chta\n")
        && str_contains($output, "bad/BrokenCard.pam.php:13:1 PAM2404 <Link> to unknown route settigns\n"),
    "pam-desktop check must report route errors:\n{$output}{$errors}",
);

[$status, , $errors] = $projectCheck($fixtures.'/clean', '--bogus');
$assert($status === 2 && str_contains($errors, 'Usage: pam-desktop check'), 'pam-desktop check must reject unknown options.');

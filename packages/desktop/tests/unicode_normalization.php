<?php

declare(strict_types=1);

// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}
// Without ext-intl, Normalizer comes from the package's polyfill dependency.
if (!class_exists(Normalizer::class)) {
    foreach ([dirname(__DIR__).'/vendor/autoload.php', dirname(__DIR__, 3).'/vendor/autoload.php'] as $autoload) {
        if (is_file($autoload)) {
            require_once $autoload;
            break;
        }
    }
}
if (!class_exists(Normalizer::class)) {
    throw new RuntimeException('Unicode normalization needs ext-intl or symfony/polyfill-intl-normalizer (composer install).');
}

$normalizationCases = [
    ["Café", "Cafe\u{0301}"],
    ["A\u{030A}\u{0301}", "A\u{030A}\u{0301}"],
    ["\u{212B}", "A\u{030A}"],
    ["각", "\u{1100}\u{1161}\u{11A8}"],
    ["\u{FB01}", "\u{FB01}"], // NFD preserves compatibility ligatures, unlike transliteration/NFKD.
];
foreach ($normalizationCases as [$source, $expected]) {
    if (Normalizer::normalize($source, Normalizer::FORM_D) !== $expected) {
        throw new RuntimeException('Canonical Unicode decomposition failed.');
    }
}
if (Normalizer::normalize("\xFF", Normalizer::FORM_D) !== false) {
    throw new RuntimeException('Malformed UTF-8 must be rejected.');
}
echo "Standard Unicode normalization and canonical/compatibility distinction: passed\n";

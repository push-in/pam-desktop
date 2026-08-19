<?php

declare(strict_types=1);

$root = dirname(__DIR__);

/** @return never */
function fail(string $message): void
{
    fwrite(STDERR, "release-workflows: {$message}\n");
    exit(1);
}

function readWorkflow(string $root, string $name): string
{
    $contents = file_get_contents("{$root}/.github/workflows/{$name}");
    if ($contents === false) {
        fail("cannot read {$name}");
    }

    return $contents;
}

function requireFragments(string $contents, string $name, array $fragments): void
{
    foreach ($fragments as $fragment) {
        if (!str_contains($contents, $fragment)) {
            fail("{$name} is missing required contract: {$fragment}");
        }
    }
}

$ci = readWorkflow($root, 'ci.yml');
$platform = readWorkflow($root, 'platform-compatibility.yml');
$release = readWorkflow($root, 'release.yml');

requireFragments($ci, 'ci.yml', ["  workflow_call:\n", "  workflow_dispatch:\n"]);
requireFragments($platform, 'platform-compatibility.yml', [
    "  workflow_call:\n",
    "      - uses: dtolnay/rust-toolchain@1.88.0\n",
    "        run: rustc --version | grep -E '^rustc 1\\.88\\.'\n",
    "            platform_code: 2\n",
    "            platform_code: 3\n",
    "      - name: Compile the real Servo desktop host\n",
]);
foreach (['ci.yml' => $ci, 'platform-compatibility.yml' => $platform, 'release.yml' => $release] as $name => $workflow) {
    if (str_contains($workflow, 'dtolnay/rust-toolchain@stable')) {
        fail("{$name} must not claim a pinned MSRV while installing the moving stable toolchain");
    }
}
requireFragments($release, 'release.yml', [
    "  source-contracts:\n",
    "    uses: ./.github/workflows/ci.yml\n",
    "  platform-contracts:\n",
    "    uses: ./.github/workflows/platform-compatibility.yml\n",
    "  build:\n    needs:\n      - native-changes\n      - platform-contracts\n      - source-contracts\n",
    "  publish-api:\n    needs:\n      - native-changes\n      - platform-contracts\n      - source-contracts\n",
]);

echo "PAM Desktop release workflow contracts passed.\n";

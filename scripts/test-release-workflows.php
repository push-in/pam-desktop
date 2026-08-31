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
$manifest = file_get_contents("{$root}/Cargo.toml");
$patchedFont = file_get_contents("{$root}/third_party/servo-fonts-0.5.0/platform/macos/font.rs");
if ($manifest === false || $patchedFont === false) {
    fail('cannot read the Servo macOS source override');
}
requireFragments($manifest, 'Cargo.toml', [
    'servo-fonts = { path = "third_party/servo-fonts-0.5.0" }',
]);
requireFragments($patchedFont, 'patched servo-fonts macOS source', [
    'Some(CFString::from_str(options.language.as_str()))',
    'language.as_deref()',
]);
if (str_contains($patchedFont, 'Some(&*CFString::from_str(options.language.as_str()))')) {
    fail('the Servo macOS source override restored the borrowed temporary');
}

requireFragments($ci, 'ci.yml', ["  workflow_call:\n", "  workflow_dispatch:\n"]);
requireFragments($platform, 'platform-compatibility.yml', [
    "  workflow_call:\n",
    "      - uses: dtolnay/rust-toolchain@1.88.0\n",
    "        run: rustc --version | grep -E '^rustc 1\\.88\\.'\n",
    "            platform_code: 2\n",
    "            platform_code: 3\n",
    "    timeout-minutes: 90\n",
    "            executable: target/release/pam-desktop\n",
    "            executable: target/release/pam-desktop.exe\n",
    "      - name: Build and smoke-test the production Servo desktop host\n",
    "          cargo build --locked --release -p pam-desktop --bins\n",
    "      - name: Build and verify the production host archive\n",
    '          name: pam-desktop-${{ matrix.target }}' . "\n",
    "      - name: Attest the native host archive\n",
    "          subject-path: |\n            dist/*.tar.gz\n            dist/*.sha256\n",
    "        run: cargo clean\n",
    "          retention-days: 14\n",
    '          path: desktop-platform-evidence-${{ matrix.platform_code }}.json' . "\n",
    "      - scripts/desktop-platform-evidence.py\n",
    "      - tests/test_desktop_platform_evidence.py\n",
]);
if (str_contains($platform, 'cargo build --locked -p pam-desktop --bins')) {
    fail('platform-compatibility.yml must not compile a redundant debug host');
}
if (substr_count($platform, 'cargo build --locked --release -p pam-desktop --bins') !== 1) {
    fail('platform-compatibility.yml must compile the production host exactly once');
}
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
    "      && needs.platform-contracts.result == 'success'\n",
    "      && needs.source-contracts.result == 'success'\n",
    "          sudo apt-get install -y curl jq libfontconfig1-dev libxkbcommon-x11-0 pkg-config xvfb\n",
    "            xvfb-run --auto-servernum --server-args='-screen 0 1280x720x24' \\\n",
]);

echo "PAM Desktop release workflow contracts passed.\n";

<?php

declare(strict_types=1);

/*
 * Runs inside PAM's Android/iOS PHP runtime (scripts/android-php-runtime-test.sh
 * packages/desktop/tests/device/crypto_runtime.php). The runtime has neither
 * ext-sodium nor ext-openssl, and this bare embed host has no
 * pam_desktop_crypto() either: signed updates and encrypted journals must fail
 * closed with CryptoUnavailableException / InvalidSignature, never with
 * "Call to undefined function sodium_*()" (PAM Desktop 1.17 and earlier).
 * The host's own implementation is covered by the Android instrumented tests
 * (NativeCryptoInstrumentedTest, NativeCryptoBridgeInstrumentedTest) and the
 * iOS XCTests (PamCryptoTests).
 */

spl_autoload_register(static function (string $class): void {
    $prefix = 'Pam\\Desktop\\';
    if (str_starts_with($class, $prefix)) {
        $path = __DIR__.'/../../src/'.str_replace('\\', '/', substr($class, strlen($prefix))).'.php';
        if (is_file($path)) {
            require $path;
        }
    }
});

use Pam\Desktop\Crypto;
use Pam\Desktop\CryptoBackend;
use Pam\Desktop\CryptoUnavailableException;
use Pam\Desktop\LocalFirst\EncryptedJournal;
use Pam\Desktop\Update\UpdateDecisionStatus;
use Pam\Desktop\Update\UpdateVerifier;

$failures = [];
foreach (['sodium', 'openssl'] as $extension) {
    if (extension_loaded($extension)) {
        $failures[] = "this runtime unexpectedly has ext-{$extension}";
    }
}
if (Crypto::ed25519Backend() !== CryptoBackend::Unavailable || Crypto::aes256GcmBackend() !== CryptoBackend::Unavailable) {
    $failures[] = 'expected no crypto backend in a bare embed host';
}
$manifest = new \Pam\Desktop\Update\SignedUpdateManifest(
    buildIdentifier: str_repeat('a', 64),
    bundleSha256: str_repeat('0', 64),
    abiVersion: \Pam\Desktop\Protocol::ABI_VERSION,
    protocolVersion: \Pam\Desktop\Protocol::VERSION,
    channel: \Pam\Desktop\Update\UpdateChannel::Stable,
    rolloutBasisPoints: 10_000,
    capabilities: ['wire.binary.v1'],
);
$decision = UpdateVerifier::evaluate(
    $manifest->canonicalJson(),
    base64_encode(str_repeat("\0", 64)),
    base64_encode(str_repeat("\0", 32)),
    str_repeat('b', 64),
    0,
);
if ($decision->status !== UpdateDecisionStatus::InvalidSignature || !str_contains($decision->message, 'pam_desktop_crypto()')) {
    $failures[] = 'an unverifiable update was not refused: '.$decision->status->name.' '.$decision->message;
}
try {
    (new EncryptedJournal(str_repeat('k', 32)))->seal([['a' => 1]]);
    $failures[] = 'a journal was sealed without AES-256-GCM';
} catch (CryptoUnavailableException) {
}
try {
    Crypto::ed25519Verify(str_repeat("\0", 64), '', str_repeat("\0", 32));
    $failures[] = 'Ed25519 verification returned without a backend';
} catch (CryptoUnavailableException) {
}
// hash_hmac()/hash_hkdf()/random_bytes() are core: what the SDK builds on.
if (hash_hkdf('sha256', str_repeat('k', 32), 32, 'info', 'salt') === '' || strlen(hash_hmac('sha256', 'm', 'k', true)) !== 32 || strlen(random_bytes(12)) !== 12) {
    $failures[] = 'ext-hash or random_bytes() is missing';
}

if ($failures !== []) {
    echo "FAIL\n".implode("\n", $failures)."\n";
    exit(1);
}
echo "crypto_runtime: ok (no sodium/openssl; updates and journals fail closed)\n";

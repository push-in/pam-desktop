<?php

declare(strict_types=1);
// Runs against the root autoloader, where bacon/bacon-qr-code is only suggested:
// a QR code without the encoder fails with an actionable message, not a fatal error.
require getenv('PAM_QR_TEST_AUTOLOAD') ?: dirname(__DIR__, 3).'/vendor/autoload.php';
use BaconQrCode\Encoder\Encoder;
use Pam\Desktop\Qr\QrCode;

if (class_exists(Encoder::class)) {
    echo 'QR without encoder: skipped, bacon/bacon-qr-code is installed.'.PHP_EOL;
    exit(0);
}
try {
    new QrCode('PAM Desktop', 256);
    throw new RuntimeException('A QR code was built without its encoder.');
} catch (LogicException $error) {
    if (!str_contains($error->getMessage(), 'composer require bacon/bacon-qr-code')) {
        throw new RuntimeException('The missing-encoder error does not say how to install it: '.$error->getMessage());
    }
}
echo 'QR without encoder: clear LogicException naming bacon/bacon-qr-code.'.PHP_EOL;

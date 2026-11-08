<?php

declare(strict_types=1);

use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModuleTransport;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Storage\SecureStorage;

$storageTransport = new class implements NativeModuleTransport {
    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void
    {
        $complete(ModuleResultStatus::Failure, 'keychain-unavailable');
    }
};
NativeModules::useTransport($storageTransport);
$loaded = false;
$storageError = null;
SecureStorage::get('test-secret', static function (?string $value) use (&$loaded): void { $loaded = true; }, static function (string $error) use (&$storageError): void { $storageError = $error; });
$assert(!$loaded && $storageError === 'keychain-unavailable', 'SecureStorage failure callback must distinguish an unavailable vault from a missing entry.');
$legacyValue = 'unchanged';
SecureStorage::get('test-secret', static function (?string $value) use (&$legacyValue): void { $legacyValue = $value; });
$assert($legacyValue === null, 'The legacy two-argument SecureStorage get remains compatible.');
NativeModules::useTransport(null);
echo "Secure storage missing-versus-unavailable callback contract passed.\n";

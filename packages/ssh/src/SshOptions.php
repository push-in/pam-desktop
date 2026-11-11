<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use DomainException;

/**
 * Where and how to connect. Give a password, a private key (OpenSSH or PEM
 * text, with its passphrase if it has one) or both: the key is tried first.
 *
 * `expectedFingerprint` is the SHA-256 fingerprint stored when the user first
 * trusted the server (`SshSession::fingerprint()`). With it, a different key
 * fails with HostKeyMismatch; without it, any key is accepted and returned so
 * the application can store it (trust on first use).
 */
final readonly class SshOptions
{
    public function __construct(
        public string $host,
        public string $username,
        public int $port = 22,
        #[\SensitiveParameter] public ?string $password = null,
        #[\SensitiveParameter] public ?string $privateKey = null,
        #[\SensitiveParameter] public ?string $passphrase = null,
        public ?string $expectedFingerprint = null,
        public int $timeoutMs = 15_000,
        public int $keepaliveSeconds = 30,
    ) {
        if ($host === '' || strlen($host) > 255 || preg_match('/[\s\x00-\x1f]/', $host) === 1) {
            throw new DomainException('SSH host must be a host name or an address.');
        }
        if ($username === '' || strlen($username) > 256) {
            throw new DomainException('SSH username must have 1 to 256 bytes.');
        }
        if ($port < 1 || $port > 65535) {
            throw new DomainException('SSH port must be between 1 and 65535.');
        }
        if (($password ?? '') === '' && ($privateKey ?? '') === '') {
            throw new DomainException('SSH needs a password or a private key.');
        }
        if (strlen($password ?? '') > 4096 || strlen($passphrase ?? '') > 4096 || strlen($privateKey ?? '') > 65536) {
            throw new DomainException('SSH credentials are too long.');
        }
        if ($timeoutMs < 1_000 || $timeoutMs > 120_000) {
            throw new DomainException('SSH timeoutMs must be between 1000 and 120000.');
        }
        if ($keepaliveSeconds < 0 || $keepaliveSeconds > 3600) {
            throw new DomainException('SSH keepaliveSeconds must be between 0 and 3600.');
        }
    }

    public static function password(string $host, string $username, #[\SensitiveParameter] string $password, int $port = 22, ?string $expectedFingerprint = null): self
    {
        return new self($host, $username, $port, password: $password, expectedFingerprint: $expectedFingerprint);
    }

    public static function key(string $host, string $username, #[\SensitiveParameter] string $privateKey, #[\SensitiveParameter] ?string $passphrase = null, int $port = 22, ?string $expectedFingerprint = null): self
    {
        return new self($host, $username, $port, privateKey: $privateKey, passphrase: $passphrase, expectedFingerprint: $expectedFingerprint);
    }

    /** @return array<string, string|int> */
    public function payload(): array
    {
        $payload = [
            'host' => $this->host,
            'port' => $this->port,
            'username' => $this->username,
            'timeoutMs' => $this->timeoutMs,
            'keepaliveSeconds' => $this->keepaliveSeconds,
        ];
        foreach (['password' => $this->password, 'privateKey' => $this->privateKey, 'passphrase' => $this->passphrase, 'expectedFingerprint' => $this->expectedFingerprint] as $key => $value) {
            if ($value !== null && $value !== '') {
                $payload[$key] = $value;
            }
        }

        return $payload;
    }

    /** Secrets stay out of var_dump() and error reports. @return array<string, mixed> */
    public function __debugInfo(): array
    {
        return [
            'host' => $this->host,
            'port' => $this->port,
            'username' => $this->username,
            'password' => $this->password === null ? null : '***',
            'privateKey' => $this->privateKey === null ? null : '***',
            'passphrase' => $this->passphrase === null ? null : '***',
            'expectedFingerprint' => $this->expectedFingerprint,
        ];
    }
}

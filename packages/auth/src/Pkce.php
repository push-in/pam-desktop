<?php

declare(strict_types=1);

namespace Pam\Desktop\Auth;

/** Proof Key for Code Exchange (RFC 7636): a verifier and its S256 challenge. */
final readonly class Pkce
{
    private function __construct(public string $verifier, public string $challenge)
    {
    }

    public static function create(): self
    {
        return self::fromVerifier(self::base64Url(random_bytes(48)));
    }

    public static function fromVerifier(string $verifier): self
    {
        return new self($verifier, self::base64Url(hash('sha256', $verifier, true)));
    }

    /** A random value for the `state` parameter. */
    public static function state(): string
    {
        return self::base64Url(random_bytes(24));
    }

    public static function base64Url(string $bytes): string
    {
        return rtrim(strtr(base64_encode($bytes), '+/', '-_'), '=');
    }
}

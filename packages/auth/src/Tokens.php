<?php

declare(strict_types=1);

namespace Pam\Desktop\Auth;

use JsonException;

/** What a token endpoint returned, with the moment the access token expires. */
final readonly class Tokens
{
    public function __construct(
        public string $accessToken,
        public ?string $refreshToken = null,
        public ?string $idToken = null,
        /** Unix time the access token expires at (null: no expiry given). */
        public ?int $expiresAt = null,
        public string $tokenType = 'Bearer',
        public ?string $scope = null,
    ) {
    }

    /**
     * @param array<string, mixed> $response the token endpoint's JSON
     * @param self|null $previous kept refresh and id tokens when a refresh omits them
     */
    public static function fromResponse(array $response, ?self $previous = null, ?int $now = null): ?self
    {
        $access = $response['access_token'] ?? null;
        if (!is_string($access) || $access === '') {
            return null;
        }
        $expiresIn = $response['expires_in'] ?? null;

        return new self(
            $access,
            is_string($response['refresh_token'] ?? null) ? $response['refresh_token'] : $previous?->refreshToken,
            is_string($response['id_token'] ?? null) ? $response['id_token'] : $previous?->idToken,
            is_numeric($expiresIn) ? ($now ?? time()) + (int) $expiresIn : null,
            is_string($response['token_type'] ?? null) ? $response['token_type'] : 'Bearer',
            is_string($response['scope'] ?? null) ? $response['scope'] : $previous?->scope,
        );
    }

    /** Expired, or expiring within `$margin` seconds. */
    public function expired(int $margin = 60, ?int $now = null): bool
    {
        return $this->expiresAt !== null && $this->expiresAt - $margin <= ($now ?? time());
    }

    /** The `Authorization` header value. */
    public function header(): string
    {
        return ucfirst(strtolower($this->tokenType)).' '.$this->accessToken;
    }

    /**
     * The claims of the OpenID id token (name, email, sub…). They are read,
     * not verified: verify the token on your server before trusting them.
     *
     * @return array<string, mixed>
     */
    public function claims(): array
    {
        $parts = explode('.', (string) $this->idToken);
        if (count($parts) !== 3) {
            return [];
        }
        $json = base64_decode(strtr($parts[1], '-_', '+/'), true);
        try {
            $claims = json_decode((string) $json, true, 32, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            return [];
        }

        return is_array($claims) ? $claims : [];
    }

    public function toJson(): string
    {
        return json_encode(get_object_vars($this), JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES);
    }

    public static function fromJson(string $json): ?self
    {
        try {
            $values = json_decode($json, true, 8, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            return null;
        }
        if (!is_array($values) || !is_string($values['accessToken'] ?? null)) {
            return null;
        }

        return new self(
            $values['accessToken'],
            is_string($values['refreshToken'] ?? null) ? $values['refreshToken'] : null,
            is_string($values['idToken'] ?? null) ? $values['idToken'] : null,
            is_int($values['expiresAt'] ?? null) ? $values['expiresAt'] : null,
            is_string($values['tokenType'] ?? null) ? $values['tokenType'] : 'Bearer',
            is_string($values['scope'] ?? null) ? $values['scope'] : null,
        );
    }
}

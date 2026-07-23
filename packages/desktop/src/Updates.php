<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Updates
{
    private function __construct(
        public string $endpoint,
        public string $channel,
        public string $publicKey,
        public UpdatePolicy $policy,
    ) {
        self::assertEndpoint($endpoint);
        Identifier::assert($channel, 'The update channel');
        if (preg_match('/\A[0-9a-f]{64}\z/D', $publicKey) !== 1) {
            throw new InvalidArgumentException(
                'The update public key must be a 32-byte lowercase hexadecimal Ed25519 key.',
            );
        }
    }

    public static function from(string $endpoint, string $publicKey): self
    {
        return new self($endpoint, 'stable', $publicKey, UpdatePolicy::Manual);
    }

    public function channel(string $channel): self
    {
        return new self($this->endpoint, $channel, $this->publicKey, $this->policy);
    }

    public function policy(UpdatePolicy $policy): self
    {
        return new self($this->endpoint, $this->channel, $this->publicKey, $policy);
    }

    /**
     * @return array{
     *     endpoint: string,
     *     channel: string,
     *     publicKey: string,
     *     policy: int
     * }
     */
    public function toArray(): array
    {
        return [
            'endpoint' => $this->endpoint,
            'channel' => $this->channel,
            'publicKey' => $this->publicKey,
            'policy' => $this->policy->value,
        ];
    }

    private static function assertEndpoint(string $endpoint): void
    {
        if (
            strlen($endpoint) > 2_048
            || preg_match('/[\x00-\x20\x7F]/u', $endpoint) === 1
            || filter_var($endpoint, FILTER_VALIDATE_URL) === false
        ) {
            throw new InvalidArgumentException('The update endpoint must be a valid HTTPS URL.');
        }

        $parts = parse_url($endpoint);
        if (
            !is_array($parts)
            || strtolower((string) ($parts['scheme'] ?? '')) !== 'https'
            || !isset($parts['host'])
            || isset($parts['user'])
            || isset($parts['pass'])
            || isset($parts['fragment'])
        ) {
            throw new InvalidArgumentException(
                'The update endpoint must use HTTPS and must not contain credentials.',
            );
        }
    }
}

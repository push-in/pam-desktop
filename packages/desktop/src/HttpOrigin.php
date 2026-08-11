<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class HttpOrigin
{
    private function __construct(
        public string $name,
        public string $origin,
    ) {
        Identifier::assert($name, 'The HTTP origin identifier');
        $parts = parse_url($origin);
        if (
            $parts === false
            || strtolower((string) ($parts['scheme'] ?? '')) !== 'https'
            || !is_string($parts['host'] ?? null)
            || $parts['host'] === ''
            || isset($parts['user'])
            || isset($parts['pass'])
            || isset($parts['query'])
            || isset($parts['fragment'])
        ) {
            throw new InvalidArgumentException(
                'Native HTTP origins must be credential-free HTTPS URLs without query or fragment.',
            );
        }
    }

    public static function allow(string $name, string $origin): self
    {
        return new self($name, rtrim($origin, '/'));
    }

    /** @return array{name: string, origin: string} */
    public function toArray(): array
    {
        return ['name' => $this->name, 'origin' => $this->origin];
    }
}

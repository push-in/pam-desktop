<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Lifecycle
{
    /**
     * @param list<string> $urlSchemes
     * @param list<string> $mimeTypes
     */
    private function __construct(
        public array $urlSchemes,
        public array $mimeTypes,
        public bool $autostartEnabled,
    ) {
        foreach ($urlSchemes as $scheme) {
            if (preg_match('/\A[a-z][a-z0-9+.-]{1,31}\z/D', $scheme) !== 1) {
                throw new InvalidArgumentException('URL schemes must be lowercase RFC 3986 schemes of 2 to 32 bytes.');
            }
        }
        foreach ($mimeTypes as $mimeType) {
            if (preg_match('~\A[a-z0-9][a-z0-9!#$&^_.+-]{0,63}/[a-z0-9][a-z0-9!#$&^_.+-]{0,63}\z~D', $mimeType) !== 1) {
                throw new InvalidArgumentException('MIME types must use a portable lowercase type/subtype form.');
            }
        }
        if (count(array_unique($urlSchemes)) !== count($urlSchemes) || count(array_unique($mimeTypes)) !== count($mimeTypes)) {
            throw new InvalidArgumentException('Lifecycle associations cannot be duplicated.');
        }
    }

    public static function none(): self
    {
        return new self([], [], false);
    }

    public function schemes(string ...$schemes): self
    {
        return new self(array_values(array_merge($this->urlSchemes, $schemes)), $this->mimeTypes, $this->autostartEnabled);
    }

    public function files(string ...$mimeTypes): self
    {
        return new self($this->urlSchemes, array_values(array_merge($this->mimeTypes, $mimeTypes)), $this->autostartEnabled);
    }

    public function autostart(bool $enabled = true): self
    {
        return new self($this->urlSchemes, $this->mimeTypes, $enabled);
    }

    /** @return array{urlSchemes: list<string>, mimeTypes: list<string>, autostart: bool} */
    public function toArray(): array
    {
        return ['urlSchemes' => $this->urlSchemes, 'mimeTypes' => $this->mimeTypes, 'autostart' => $this->autostartEnabled];
    }
}

<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Manifest
{
    /**
     * @param list<string> $bundleExcludes
     */
    private function __construct(
        public string $identifier,
        public string $name,
        public string $version,
        public string $description,
        public string $publisher,
        public ApplicationCategory $category,
        public string $icon,
        public array $bundleExcludes,
    ) {
        self::assertIdentifier($identifier);
        self::assertText($name, 'The application name', 80);
        self::assertVersion($version);
        self::assertText($description, 'The application description', 256, true);
        self::assertText($publisher, 'The application publisher', 128);
        self::assertProjectPath($icon, 'The application icon');
        if (!in_array(strtolower(pathinfo($icon, PATHINFO_EXTENSION)), ['png', 'svg'], true)) {
            throw new InvalidArgumentException('The application icon must be a PNG or SVG asset.');
        }

        $unique = [];
        foreach ($bundleExcludes as $path) {
            self::assertProjectPath($path, 'A bundle exclusion');
            if (isset($unique[$path])) {
                throw new InvalidArgumentException("The bundle exclusion {$path} is duplicated.");
            }
            if (in_array($path, ['app.php', 'composer.json', 'composer.lock', 'resources', 'vendor'], true)) {
                throw new InvalidArgumentException("The required project path {$path} cannot be excluded.");
            }
            $unique[$path] = true;
        }
    }

    public static function create(string $identifier, string $name, string $version): self
    {
        return new self(
            identifier: $identifier,
            name: $name,
            version: $version,
            description: '',
            publisher: $name,
            category: ApplicationCategory::Utility,
            icon: 'resources/icon.svg',
            bundleExcludes: [],
        );
    }

    public function description(string $description): self
    {
        return new self(
            $this->identifier,
            $this->name,
            $this->version,
            $description,
            $this->publisher,
            $this->category,
            $this->icon,
            $this->bundleExcludes,
        );
    }

    public function publisher(string $publisher): self
    {
        return new self(
            $this->identifier,
            $this->name,
            $this->version,
            $this->description,
            $publisher,
            $this->category,
            $this->icon,
            $this->bundleExcludes,
        );
    }

    public function category(ApplicationCategory $category): self
    {
        return new self(
            $this->identifier,
            $this->name,
            $this->version,
            $this->description,
            $this->publisher,
            $category,
            $this->icon,
            $this->bundleExcludes,
        );
    }

    public function icon(string $path): self
    {
        return new self(
            $this->identifier,
            $this->name,
            $this->version,
            $this->description,
            $this->publisher,
            $this->category,
            $path,
            $this->bundleExcludes,
        );
    }

    public function excludeFromBundle(string ...$paths): self
    {
        return new self(
            $this->identifier,
            $this->name,
            $this->version,
            $this->description,
            $this->publisher,
            $this->category,
            $this->icon,
            array_values(array_merge($this->bundleExcludes, $paths)),
        );
    }

    /**
     * @return array{
     *     identifier: string,
     *     name: string,
     *     version: string,
     *     description: string,
     *     publisher: string,
     *     category: int,
     *     icon: string,
     *     bundleExcludes: list<string>
     * }
     */
    public function toArray(): array
    {
        return [
            'identifier' => $this->identifier,
            'name' => $this->name,
            'version' => $this->version,
            'description' => $this->description,
            'publisher' => $this->publisher,
            'category' => $this->category->value,
            'icon' => $this->icon,
            'bundleExcludes' => $this->bundleExcludes,
        ];
    }

    private static function assertIdentifier(string $identifier): void
    {
        if (
            strlen($identifier) > 155
            || preg_match('/\A[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+\z/D', $identifier) !== 1
        ) {
            throw new InvalidArgumentException(
                'The application identifier must be a lowercase reverse-DNS name of at most 155 bytes.',
            );
        }
    }

    private static function assertVersion(string $version): void
    {
        if (preg_match('/\A[0-9][A-Za-z0-9.+~-]{0,63}\z/D', $version) !== 1) {
            throw new InvalidArgumentException(
                'The application version must begin with a number and use at most 64 portable characters.',
            );
        }
    }

    private static function assertText(
        string $value,
        string $label,
        int $maximumBytes,
        bool $allowEmpty = false,
    ): void {
        if (
            (!$allowEmpty && trim($value) === '')
            || strlen($value) > $maximumBytes
            || preg_match('/[\x00-\x1F\x7F]/u', $value) !== 0
        ) {
            throw new InvalidArgumentException(
                "{$label} must contain printable UTF-8 text of at most {$maximumBytes} bytes.",
            );
        }
    }

    private static function assertProjectPath(string $path, string $label): void
    {
        if (
            $path === ''
            || str_contains($path, "\0")
            || str_starts_with($path, '/')
            || str_starts_with($path, '\\')
            || preg_match('/\A[A-Za-z]:[\\\\\/]/D', $path) === 1
            || preg_match('~(^|[\\\\/])\.\.([\\\\/]|$)~', $path) === 1
        ) {
            throw new InvalidArgumentException("{$label} must be a relative project path.");
        }
    }
}

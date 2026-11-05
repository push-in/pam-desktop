<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class GlobalShortcut
{
    private function __construct(
        public string $id,
        public string $accelerator,
    ) {
        Identifier::assert($id, 'The global shortcut identifier');
        Accelerator::assert($accelerator);
    }

    public static function create(string $id, string $accelerator): self
    {
        return new self($id, $accelerator);
    }

    /**
     * @return array{id: string, accelerator: string}
     */
    public function toArray(): array
    {
        return [
            'id' => $this->id,
            'accelerator' => $this->accelerator,
        ];
    }
}

<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class QuickAction
{
    private function __construct(
        public string $id,
        public string $label,
        public string $description,
    ) {
        Identifier::assert($id, 'The quick action identifier');
        if (trim($label) === '' || strlen($label) > 80 || preg_match('/[\x00-\x1F\x7F]/', $label) === 1) {
            throw new InvalidArgumentException('The quick action label must contain 1 to 80 printable bytes.');
        }
        if (strlen($description) > 160 || preg_match('/[\x00-\x1F\x7F]/', $description) === 1) {
            throw new InvalidArgumentException('The quick action description must contain at most 160 printable bytes.');
        }
    }

    public static function create(string $id, string $label): self
    {
        return new self($id, $label, '');
    }

    public function description(string $description): self
    {
        return new self($this->id, $this->label, $description);
    }

    /** @return array{id: string, label: string, description: string} */
    public function toArray(): array
    {
        return [
            'id' => $this->id,
            'label' => $this->label,
            'description' => $this->description,
        ];
    }
}

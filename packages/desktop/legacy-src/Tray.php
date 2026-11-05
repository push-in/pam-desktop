<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class Tray
{
    private function __construct(
        public string $menuId,
        public string $tooltip,
        public TrayCloseBehavior $closeBehavior,
    ) {
        Identifier::assert($menuId, 'The tray menu identifier');
        if (trim($tooltip) === '' || strlen($tooltip) > 128) {
            throw new InvalidArgumentException(
                'The tray tooltip must contain at most 128 bytes of printable text.',
            );
        }
    }

    public static function create(string $menuId, string $tooltip): self
    {
        return new self($menuId, $tooltip, TrayCloseBehavior::Exit);
    }

    public function closeBehavior(TrayCloseBehavior $behavior): self
    {
        return new self($this->menuId, $this->tooltip, $behavior);
    }

    /**
     * @return array{menuId: string, tooltip: string, closeBehavior: int}
     */
    public function toArray(): array
    {
        return [
            'menuId' => $this->menuId,
            'tooltip' => $this->tooltip,
            'closeBehavior' => $this->closeBehavior->value,
        ];
    }
}

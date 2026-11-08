<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

final readonly class AutocompleteOption
{
    public string $key;
    public function __construct(public int|string $value, public string $label, public string $detail = '', public string $image = '', public bool $disabled = false)
    {
        $this->key = (is_int($value) ? 'i:' : 's:').$value;
    }
}

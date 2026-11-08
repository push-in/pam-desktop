<?php

declare(strict_types=1);

namespace Pam\Desktop\Forms\Attributes;

use Attribute;
use Pam\Desktop\Forms\NativeForm;
use Pam\Desktop\Forms\ValidationRule;

#[Attribute(Attribute::TARGET_PROPERTY)]
final readonly class Matches implements ValidationRule
{
    public function __construct(
        public string $other,
        public ?string $message = null,
    ) {
    }

    public function validate(string $field, mixed $value, NativeForm $form): ?string
    {
        unset($field);

        return $form->value($this->other) === $value ? null : ($this->message ?? \Pam\Desktop\I18n\FrameworkText::get('form.matches'));
    }
}

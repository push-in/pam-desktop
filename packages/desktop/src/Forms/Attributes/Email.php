<?php

declare(strict_types=1);

namespace Pam\Desktop\Forms\Attributes;

use Attribute;
use Pam\Desktop\Forms\NativeForm;
use Pam\Desktop\Forms\ValidationRule;

use function is_string;

#[Attribute(Attribute::TARGET_PROPERTY)]
final readonly class Email implements ValidationRule
{
    public function __construct(
        public ?string $message = null,
    ) {
    }

    public function validate(string $field, mixed $value, NativeForm $form): ?string
    {
        unset($field, $form);

        if ($value === null || $value === '') {
            return null;
        }

        return is_string($value) && filter_var($value, FILTER_VALIDATE_EMAIL) !== false
            ? null
            : ($this->message ?? \Pam\Desktop\I18n\FrameworkText::get('form.email'));
    }
}

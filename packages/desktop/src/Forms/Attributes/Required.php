<?php

declare(strict_types=1);

namespace Pam\Desktop\Forms\Attributes;

use Attribute;
use Pam\Desktop\Forms\NativeForm;
use Pam\Desktop\Forms\ValidationRule;

use function is_array;
use function is_string;

#[Attribute(Attribute::TARGET_PROPERTY)]
final readonly class Required implements ValidationRule
{
    public function __construct(
        public string $message = 'This field is required.',
    ) {
    }

    public function validate(string $field, mixed $value, NativeForm $form): ?string
    {
        unset($field, $form);

        return $value === null
            || is_string($value) && trim($value) === ''
            || is_array($value) && $value === []
                ? $this->message
                : null;
    }
}

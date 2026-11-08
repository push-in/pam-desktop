<?php

declare(strict_types=1);

namespace Pam\Desktop\Forms;

interface ValidationRule
{
    public function validate(string $field, mixed $value, NativeForm $form): ?string;
}

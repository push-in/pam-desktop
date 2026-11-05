<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

/**
 * Explanation shown in a native alert before `Permissions::requestKind()`
 * asks again, when the platform says a rationale should be shown. Never
 * shown on desktop, where every kind is already granted.
 */
final readonly class PermissionRationale
{
    public function __construct(
        public string $title,
        public string $message,
        public string $confirmLabel,
        public string $cancelLabel,
    ) {
        if (trim($confirmLabel) === '' || trim($cancelLabel) === '') {
            throw new InvalidArgumentException('Permission rationale buttons need labels.');
        }
    }
}

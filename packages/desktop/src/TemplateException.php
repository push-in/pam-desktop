<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;
use Throwable;

final class TemplateException extends RuntimeException
{
    /** The message without its location. */
    public readonly string $reason;

    public function __construct(
        string $message,
        public readonly string $template,
        public readonly int $templateLine,
        public readonly int $templateColumn = 1,
        /** The error an expression or a component of the template raised. */
        ?Throwable $previous = null,
    ) {
        $this->reason = $message;
        parent::__construct("{$message} at {$template}:{$templateLine}:{$templateColumn}", 0, $previous);
    }
}

<?php

declare(strict_types=1);

namespace Pam\Desktop\Scheduling;

final class DeadlineExceededException extends \RuntimeException
{
    public function __construct()
    {
        parent::__construct('Structured task deadline exceeded.');
    }
}

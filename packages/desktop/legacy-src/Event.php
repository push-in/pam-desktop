<?php

declare(strict_types=1);

namespace Pam\Desktop;

interface Event
{
    public function name(): string;

    public function payload(): mixed;
}

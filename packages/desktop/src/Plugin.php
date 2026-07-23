<?php

declare(strict_types=1);

namespace Pam\Desktop;

interface Plugin
{
    public function identifier(): string;

    public function register(Application $application): void;
}

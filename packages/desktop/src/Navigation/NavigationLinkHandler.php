<?php

declare(strict_types=1);

namespace Pam\Desktop\Navigation;

interface NavigationLinkHandler
{
    public function open(string $uri): bool;

    public function currentPath(): ?string;

    public function currentUrl(): ?string;
}

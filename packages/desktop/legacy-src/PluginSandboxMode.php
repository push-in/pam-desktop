<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum PluginSandboxMode: int
{
    case Inherited = 1;
    case Strict = 2;
}

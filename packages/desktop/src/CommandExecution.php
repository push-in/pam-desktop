<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum CommandExecution: int
{
    case Stateful = 1;
    case Parallel = 2;
    case Background = 3;
}

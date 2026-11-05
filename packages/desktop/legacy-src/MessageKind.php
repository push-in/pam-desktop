<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum MessageKind: int
{
    case Request = 1;
    case Response = 2;
}


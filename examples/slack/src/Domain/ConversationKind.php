<?php

declare(strict_types=1);

namespace App\Domain;

enum ConversationKind
{
    case Channel;
    case Private;
    case Direct;
}

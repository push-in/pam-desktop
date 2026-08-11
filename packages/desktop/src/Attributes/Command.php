<?php

declare(strict_types=1);

namespace Pam\Desktop\Attributes;

use Attribute;
use Pam\Desktop\CommandExecution;

#[Attribute(Attribute::TARGET_METHOD | Attribute::TARGET_CLASS)]
final readonly class Command
{
    public function __construct(
        public ?string $name = null,
        public CommandExecution $execution = CommandExecution::Stateful,
    ) {
    }
}

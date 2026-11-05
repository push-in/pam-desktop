<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\Fixtures\Domain;

enum TemplateState: int
{
    case Ready = 1;
}

namespace Pam\Desktop\Tests\Fixtures\Screen;

use Pam\Desktop\Tests\Fixtures\Domain\TemplateState as ImportedState;

final class TemplateEnumScope
{
}

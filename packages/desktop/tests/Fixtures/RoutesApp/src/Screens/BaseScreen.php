<?php

declare(strict_types=1);

namespace RoutesApp\Screens;

use Pam\Desktop\Element;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Screen;
use Pam\Desktop\UI\Text;

abstract class BaseScreen implements Renderable
{
    public function toElement(): Element
    {
        return Screen::make(Text::make(static::class))->toElement();
    }
}

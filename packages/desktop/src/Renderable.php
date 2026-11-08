<?php

declare(strict_types=1);

namespace Pam\Desktop;

interface Renderable
{
    public function toElement(): Element;
}

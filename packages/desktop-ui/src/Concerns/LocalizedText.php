<?php

declare(strict_types=1);

namespace Pam\DesktopUi\Concerns;

use Pam\Desktop\I18n\FrameworkText;

/** Framework default copy for templates: `uiText('select.clear')` (see FrameworkText). */
trait LocalizedText
{
    /** @param array<string, string|int|float> $params */
    public function uiText(string $key, array $params = []): string
    {
        return FrameworkText::get($key, $params);
    }
}

<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Component;

/** Centers short dialogs and scrolls tall content without losing either edge. */
final class DialogViewport extends Component
{
    public function __construct(public int $inset = 16)
    {
        if ($inset < 0 || $inset > 128) {
            throw new \InvalidArgumentException('Dialog inset must be between 0 and 128 pixels.');
        }
    }
}
?>
<template>
    <ScrollView width="100%" height="100%" fillViewport="true">
        <Column width="100%" minHeight="100%" :padding="$inset" alignItems="center" justifyContent="center">
            <Slot />
        </Column>
    </ScrollView>
</template>

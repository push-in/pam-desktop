<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Component;

/** @internal The window layer content of ToastService: the stack and its portal host. */
final class ToastOutlet extends Component
{
    public function __construct(public readonly Toast $toast)
    {
    }
}
?>
<template>
    <View position="absolute" left="0" top="0" right="0" bottom="0" pointerEvents="box-none">
        <Slot :content="$toast" />
        <PortalHost :name="$toast->host" />
    </View>
</template>

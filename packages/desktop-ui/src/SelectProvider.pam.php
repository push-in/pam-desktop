<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Component;
use Pam\Desktop\System\ViewRef;

final class SelectProvider extends Component
{
    public function __construct(public string $host = 'desktop-ui')
    {
        ViewRef::of($host);
    }
}
?>
<template>
    <View :nativeRef="$host" flexGrow="1">
        <Slot />
        <PortalHost :name="$host" />
    </View>
</template>

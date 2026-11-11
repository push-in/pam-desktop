<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\ProjectCheck\Broken\Screens;

use Pam\Desktop\Component;

final class Totals extends Component
{
    public int $count = 2;

    public function label(): string
    {
        return "{$this->count} items";
    }
}
?>

<template>
    <Column>
        <Text>{{ $count + }}</Text>
        <Text p-if="missing() && $count > 0">{{ label() }} · {{ strtoupper(label()) }}</Text>
        <Text :numberOfLines="max($count, 1)">{{ $this->label() }}</Text>
    </Column>
</template>

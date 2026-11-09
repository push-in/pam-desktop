<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\ProjectCheck\Broken;

use Pam\Desktop\Attributes\Action;
use Pam\Desktop\Component;

final class Greeting extends Component
{
    public int $count = 0;

    #[Action]
    public function increment(): void
    {
        $this->count++;
    }
}
?>

<template language="2">
    <Column class="card">
        <Text textColor="#222222">Hello {{ $count }}</Text>
        <Pressable on:press="increment"><Text>More</Text></Pressable>
    </Column>
</template>

<style>
.card { padding: 12px; background-color: #ffffff; }
</style>

<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\ProjectCheck\Broken\Screens;

use Pam\Desktop\Component;

final class Profile extends Component
{
    public string $name = 'Ada';
}
?>

<template language="2">
    <Column>
        <Text color="#222222">{{ $name }}</Text>
        <Pressable on:press="save"><Text maxLines="1">Save</Text></Pressable>
        <Text class="card ghost">Unstyled</Text>
        <Row :class="$name === 'Ada' ? 'card' : 'missing'" />
    </Column>
</template>

<style>
.card { padding: 12px; }
</style>

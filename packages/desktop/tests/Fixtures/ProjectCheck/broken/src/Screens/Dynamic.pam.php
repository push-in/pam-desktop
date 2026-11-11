<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\ProjectCheck\Broken\Screens;

use Pam\Desktop\Component;

final class Dynamic extends Component
{
    public string $name = 'a';

    public function value(string $field): string
    {
        return (string) $this->{$field};
    }
}
?>

<template>
    <Text>{{ $name }}</Text>
</template>

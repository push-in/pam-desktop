<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\Fixtures;

use Pam\Desktop\Plugin\PluginProvider;
use Pam\Desktop\TemplateRegistry;
use Pam\Desktop\UI\Text;

final class ExamplePluginProvider implements PluginProvider
{
    public static int $registered = 0;
    public static int $booted = 0;

    public function register(): void
    {
        self::$registered++;
        TemplateRegistry::component(
            'FixtureTag',
            static fn (array $_props, array $_children, ?object $_scope): Text => Text::make('Plugin fixture'),
        );
    }

    public function boot(): void
    {
        self::$booted++;
    }
}

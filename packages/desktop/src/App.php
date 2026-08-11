<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Attributes\Command as CommandAttribute;
use Pam\Desktop\Attributes\Desktop as DesktopAttribute;
use Pam\Desktop\Attributes\Menu as MenuAttribute;
use Pam\Desktop\Attributes\MenuItem as MenuItemAttribute;
use Pam\Desktop\Attributes\MenuSeparator;
use Pam\Desktop\Attributes\Listen as ListenAttribute;
use Pam\Desktop\Attributes\Window as WindowAttribute;
use ReflectionClass;
use ReflectionMethod;
use RuntimeException;

abstract class App
{
    final public static function run(): never
    {
        static::application()->run();
    }

    final public static function application(?Container $container = null): Application
    {
        $container ??= new Container();
        foreach (static::bindings() as $id => $binding) {
            $container->bind($id, $binding);
        }
        $reflection = new ReflectionClass(static::class);
        $attributes = $reflection->getAttributes(DesktopAttribute::class);
        if (count($attributes) !== 1) {
            throw new RuntimeException(
                static::class.' must declare exactly one #[Desktop] attribute.',
            );
        }

        /** @var DesktopAttribute $metadata */
        $metadata = $attributes[0]->newInstance();
        $window = Window::create($metadata->name)
            ->load($metadata->page)
            ->size($metadata->width, $metadata->height)
            ->minimumSize($metadata->minimumWidth, $metadata->minimumHeight)
            ->theme($metadata->theme);
        $application = Application::make(
            id: $metadata->id,
            name: $metadata->name,
            version: $metadata->version,
            window: $window,
        )
            ->category($metadata->category)
            ->publisher($metadata->publisher ?? $metadata->name);
        if ($metadata->description !== '') {
            $application->description($metadata->description);
        }

        /** @var static $app */
        $app = $container->get(static::class);
        $desktop = new Desktop($application);
        $app->configure($desktop);
        $app->registerWindows($desktop);

        $invoker = new CommandInvoker($container, $desktop->windowIds(), $desktop->windowClasses());
        $app->registerCommands($application, $invoker, $app);
        $app->registerListeners($application, $invoker, $app);
        foreach ($app->commands() as $command) {
            $target = is_string($command) ? $container->get($command) : $command;
            $app->registerCommands($application, $invoker, $target);
            $app->registerListeners($application, $invoker, $target);
        }
        $app->registerMenus($application, $invoker, $container);

        return $application;
    }

    protected function configure(Desktop $desktop): void
    {
    }

    /** @return array<class-string, object> */
    protected static function bindings(): array
    {
        return [];
    }

    /** @return list<class-string|object> */
    protected function commands(): array
    {
        return [];
    }

    /** @return list<class-string> */
    protected function windows(): array
    {
        return [];
    }

    /** @return list<class-string|object> */
    protected function menus(): array
    {
        return [];
    }

    private function registerWindows(Desktop $desktop): void
    {
        foreach ($this->windows() as $class) {
            $reflection = new ReflectionClass($class);
            $attributes = $reflection->getAttributes(WindowAttribute::class);
            if (count($attributes) !== 1) {
                throw new RuntimeException("{$class} must declare exactly one #[Window] attribute.");
            }
            /** @var WindowAttribute $metadata */
            $metadata = $attributes[0]->newInstance();
            if (!$reflection->isSubclassOf(DesktopWindow::class)) {
                throw new RuntimeException("{$class} must extend ".DesktopWindow::class.'.');
            }
            /** @var class-string<DesktopWindow> $class */
            $desktop->windowClass(
                $class,
                $metadata->name,
                Window::create($metadata->title)
                    ->load($metadata->page)
                    ->size($metadata->width, $metadata->height)
                    ->minimumSize($metadata->minimumWidth, $metadata->minimumHeight)
                    ->visible($metadata->visible)
                    ->theme($metadata->theme),
            );
        }
    }

    private function registerCommands(
        Application $application,
        CommandInvoker $invoker,
        object $target,
    ): void {
        $reflection = new ReflectionClass($target);
        $classAttributes = $reflection->getAttributes(CommandAttribute::class);
        if ($classAttributes !== []) {
            if (!$reflection->hasMethod('__invoke')) {
                throw new RuntimeException("Attributed command {$reflection->getName()} must be invokable.");
            }
            /** @var CommandAttribute $attribute */
            $attribute = $classAttributes[0]->newInstance();
            $this->registerCommand($application, $invoker, $target, '__invoke', $attribute);
        }

        foreach ($reflection->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
            foreach ($method->getAttributes(CommandAttribute::class) as $attribute) {
                /** @var CommandAttribute $command */
                $command = $attribute->newInstance();
                $this->registerCommand($application, $invoker, $target, $method->getName(), $command);
            }
        }
    }

    private function registerCommand(
        Application $application,
        CommandInvoker $invoker,
        object $target,
        string $method,
        CommandAttribute $attribute,
    ): void {
        $name = $attribute->name ?? self::commandName($method === '__invoke'
            ? (new ReflectionClass($target))->getShortName()
            : $method);
        $application->command(
            $name,
            static fn (CommandContext $context): CommandResult => $invoker->invoke($target, $method, $context),
            $attribute->execution,
        );
    }

    private function registerListeners(
        Application $application,
        CommandInvoker $invoker,
        object $target,
    ): void {
        $reflection = new ReflectionClass($target);
        foreach ($reflection->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
            foreach ($method->getAttributes(ListenAttribute::class) as $attribute) {
                /** @var ListenAttribute $listener */
                $listener = $attribute->newInstance();
                $name = $listener->event ?? self::commandName($method->getName());
                $application->on(
                    $name,
                    static fn (EventContext $context): CommandResult => $invoker->invokeEvent(
                        $target,
                        $method->getName(),
                        $context,
                    ),
                );
            }
        }
    }

    private function registerMenus(
        Application $application,
        CommandInvoker $invoker,
        Container $container,
    ): void {
        $menus = [];
        $shortcuts = [];
        $tray = null;
        foreach ($this->menus() as $menuDefinition) {
            $target = is_string($menuDefinition) ? $container->get($menuDefinition) : $menuDefinition;
            $reflection = new ReflectionClass($target);
            $attributes = $reflection->getAttributes(MenuAttribute::class);
            if (count($attributes) !== 1) {
                throw new RuntimeException("{$reflection->getName()} must declare exactly one #[Menu] attribute.");
            }
            /** @var MenuAttribute $metadata */
            $metadata = $attributes[0]->newInstance();
            $items = [];
            foreach ($reflection->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
                if ($method->getAttributes(MenuSeparator::class) !== []) {
                    $items[] = MenuItem::separator();
                }
                foreach ($method->getAttributes(MenuItemAttribute::class) as $itemAttribute) {
                    /** @var MenuItemAttribute $item */
                    $item = $itemAttribute->newInstance();
                    $id = $item->id ?? self::commandName($method->getName());
                    $items[] = $item->checkbox
                        ? MenuItem::checkbox($id, $item->label, $item->checked, $item->shortcut)
                        : MenuItem::command($id, $item->label, $item->shortcut);
                    if ($item->shortcut !== null) {
                        $shortcuts[] = GlobalShortcut::create($id, $item->shortcut);
                    }
                    $this->registerCommand(
                        $application,
                        $invoker,
                        $target,
                        $method->getName(),
                        new CommandAttribute($id),
                    );
                }
            }
            $menus[] = Menu::create($metadata->id, $metadata->label, ...$items);
            if ($metadata->tray) {
                if ($tray !== null) {
                    throw new RuntimeException('Only one attributed tray menu is supported on Linux.');
                }
                $tray = Tray::create($metadata->id, $metadata->tooltip ?? $metadata->label)
                    ->closeBehavior($metadata->close);
            }
        }
        if ($menus !== []) {
            $shell = Shell::none()->menu(...$menus);
            if ($tray !== null) {
                $shell = $shell->tray($tray);
            }
            if ($shortcuts !== []) {
                $shell = $shell->shortcut(...$shortcuts);
            }
            $application->shell($shell);
        }
    }

    private static function commandName(string $name): string
    {
        return strtolower(preg_replace('/(?<!^)[A-Z]/', '.$0', $name) ?? $name);
    }
}

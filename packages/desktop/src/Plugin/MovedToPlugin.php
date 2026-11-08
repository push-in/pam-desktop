<?php

declare(strict_types=1);

namespace Pam\Desktop\Plugin;

use function class_alias;
use function class_exists;
use function enum_exists;
use function interface_exists;
use function sprintf;

/**
 * Forwards a class name of the shared SDK to the optional package that
 * implements it.
 *
 * Capabilities that only exist on phones (location, contacts, sensors,
 * biometrics…) are not part of the desktop core. Their class names stay
 * known so shared code fails with a clear message: with the package
 * installed the name is an alias of its class (same class, so `instanceof`,
 * enum cases and type hints keep working); without it, autoloading the name
 * throws a `PluginException` that names the package.
 */
final class MovedToPlugin
{
    /** @var array<string, string> legacy class => plugin package */
    private static array $aliased = [];

    private function __construct()
    {
    }

    public static function alias(string $legacy, string $target, string $package): void
    {
        if (!class_exists($target) && !interface_exists($target) && !enum_exists($target)) {
            throw new PluginException(sprintf(
                '%s is not part of PAM Desktop: it belongs to the optional package %s (%s), which is not installed.',
                $legacy,
                $package,
                $target,
            ));
        }
        if (!isset(self::$aliased[$legacy])) {
            class_alias($target, $legacy);
            self::$aliased[$legacy] = $package;
        }
    }

    /** @return array<string, string> legacy class => plugin package, for doctor/audit output. */
    public static function aliased(): array
    {
        return self::$aliased;
    }
}

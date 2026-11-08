<?php

declare(strict_types=1);

namespace Pam\Desktop\Routing;

use Closure;
use Pam\Desktop\Navigation\ScreenOptions;
use Pam\Desktop\Navigation\ScreenOptionsPatch;

/** @internal */
final class RouteDefinition
{
    /** @var list<ScreenOptions|ScreenOptionsPatch|Closure> */
    public array $groupOptions = [];
    /** @var list<Closure> */
    public array $groupGuards = [];
    /** @var list<ScreenOptions|ScreenOptionsPatch|Closure> */
    public array $options = [];
    public ?Closure $guard = null;
    public ?Closure $getId = null;
    public bool|Closure $keepAlive = false;
    public ?Closure $errorBoundary = null;
    /** @var list<string> */
    public array $deepLinks = [];

    public function __construct(
        public readonly string $name,
        public readonly Closure $factory,
    ) {
    }
}

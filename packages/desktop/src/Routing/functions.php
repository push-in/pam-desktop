<?php

declare(strict_types=1);

namespace Pam\Desktop;

use BackedEnum;
use Pam\Desktop\Navigation\NavigationTransition;
use Pam\Desktop\Routing\Redirect;
use Pam\Desktop\Routing\RedirectMode;
use Pam\Desktop\Routing\TypedRoute;

if (!function_exists('Pam\\Desktop\\navigate')) {
    /**
     * Navigate by route name, screen class or generated typed route:
     * `navigate('chat', ['id' => $id])`, `navigate(ChatScreen::class, id: $id)`,
     * `navigate(Routes::chat(id: $id))`. Middleware runs first.
     *
     * `mode:` 'navigate' (default: back to the entry when it is in the stack,
     * else push), 'push', 'replace' or 'reset'; `transition:`/`duration:`
     * override the route's animation for this step; `merge: true` keeps the
     * params of the route's entry that are not given. `mode`, `transition`,
     * `merge` and `duration` are therefore reserved: a route param with one of
     * those names goes in the $params array.
     *
     * @param array<string, string|int|float|bool|null> $params
     */
    function navigate(
        string|BackedEnum|TypedRoute $route,
        array $params = [],
        RedirectMode|string $mode = RedirectMode::Navigate,
        ?NavigationTransition $transition = null,
        bool $merge = false,
        ?int $duration = null,
        string|int|float|bool|null ...$named,
    ): bool {
        return Route::navigate($route, [...$params, ...$named], $mode, $transition, $merge, $duration);
    }
}

if (!function_exists('Pam\\Desktop\\back')) {
    function back(): bool
    {
        return Route::back();
    }
}

if (!function_exists('Pam\\Desktop\\redirect')) {
    /**
     * For middleware: `return redirect('login');` (`->reset()`, `->replace()`,
     * `->with([...])` adjust it).
     *
     * @param array<string, string|int|float|bool|null> $params
     */
    function redirect(string|BackedEnum|TypedRoute $route, array $params = [], string|int|float|bool|null ...$named): Redirect
    {
        return new Redirect($route, [...$params, ...$named]);
    }
}

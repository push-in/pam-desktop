<?php

declare(strict_types=1);

use Pam\Desktop\Host\Channel;

/*
 * Host entry points for the desktop worker process: they forward to the
 * native host over the worker channel. Loaded only by the worker bootstrap,
 * so tests and tooling keep seeing a runtime without a host.
 */

if (!function_exists('pam_desktop_commit')) {
    function pam_desktop_commit(string $frame): bool
    {
        Channel::send(Channel::COMMIT, $frame);
        Channel::flush();

        // The host applies frames asynchronously. A rejected patch comes back
        // as RESYNC, which re-encodes a complete tree.
        return true;
    }
}

if (!function_exists('pam_desktop_call')) {
    function pam_desktop_call(int $requestId, string $module, string $method, string $payload): void
    {
        Channel::send(
            Channel::CALL,
            pack('Pvv', $requestId, strlen($module), strlen($method)).$module.$method.$payload,
        );
    }
}

if (!function_exists('pam_desktop_call_typed')) {
    function pam_desktop_call_typed(int $requestId, int $operation, string $payload): void
    {
        Channel::send(Channel::CALL_TYPED, pack('Pv', $requestId, $operation).$payload);
    }
}

if (!function_exists('pam_desktop_error')) {
    function pam_desktop_error(string $message): void
    {
        Channel::send(Channel::ERROR, $message);
        Channel::flush();
    }
}

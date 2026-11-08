<?php

declare(strict_types=1);

namespace Pam\Desktop\Realtime;

use InvalidArgumentException;

/** The bearer authorization origin is derived only from the caller's trusted API URL. */
final readonly class ReverbConfig
{
    public string $authorizationUrl;
    public string $socketUrl;
    public function __construct(string $trustedApiUrl, string $socketBaseUrl, public string $publicKey, string $authorizationPath = '/broadcasting/auth')
    {
        $api = parse_url($trustedApiUrl);
        $socket = parse_url($socketBaseUrl);
        if ($api === false || $socket === false || !isset($api['scheme'],$api['host'],$socket['scheme'],$socket['host']) || isset($api['user']) || isset($api['pass']) || isset($socket['user']) || isset($socket['pass']) || isset($socket['query']) || isset($socket['fragment']) || preg_match('/^[A-Za-z0-9_-]{1,200}$/D', $publicKey) !== 1 || preg_match('~^/[A-Za-z0-9_/-]+$~D', $authorizationPath) !== 1 || str_starts_with($authorizationPath, '//')) {
            throw new InvalidArgumentException('Invalid realtime configuration.');
        }
        foreach ([$trustedApiUrl,$socketBaseUrl] as $url) {
            if (strlen($url) > 2048 || preg_match('/[\x00-\x20\x7f\\\\]/', $url)) {
                throw new InvalidArgumentException('Invalid realtime URL.');
            }
        }
        if ($api['scheme'] !== 'https' && !($api['scheme'] === 'http' && self::loopback($api['host']))) {
            throw new InvalidArgumentException('Realtime authorization requires HTTPS.');
        }
        if ($socket['scheme'] !== 'wss' && !($socket['scheme'] === 'ws' && self::loopback($socket['host']))) {
            throw new InvalidArgumentException('Realtime requires WSS.');
        }
        if (($socket['path'] ?? '') !== '' && $socket['path'] !== '/') {
            throw new InvalidArgumentException('The WebSocket base URL must be an origin.');
        }
        $origin = $api['scheme'].'://'.$api['host'].(isset($api['port']) ? ':'.$api['port'] : '');
        $this->authorizationUrl = $origin.$authorizationPath;
        $this->socketUrl = rtrim($socketBaseUrl, '/').'/app/'.$publicKey.'?protocol=7&client=pam-desktop&version=1.0&flash=false';
    }
    private static function loopback(string $host): bool
    {
        return in_array(strtolower($host), ['localhost','127.0.0.1','[::1]'], true);
    }
}

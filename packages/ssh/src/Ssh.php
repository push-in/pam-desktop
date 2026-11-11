<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use Pam\Desktop\Component;

/**
 * SSH sessions in the host. Connecting, encryption and transfers run on the
 * host's `pam-ssh` threads; PHP only hears results and events.
 *
 * ```php
 * $session = Ssh::connect(
 *     SshOptions::password('server.example', 'deploy', $password, expectedFingerprint: $saved),
 *     function (SshSession $session): void { $this->fingerprint = $session->fingerprint(); },
 *     function (SshException $error): void { $this->error = $error->getMessage(); },
 *     owner: $this,
 * );
 * ```
 */
final class Ssh
{
    private function __construct()
    {
    }

    /**
     * @param Closure(SshSession): void $connected
     * @param Closure(SshException): void $failed HostKeyMismatch, AuthFailed or SshException.
     * @param Component|null $owner The session is disconnected when the component unmounts.
     */
    public static function connect(SshOptions $options, Closure $connected, Closure $failed, ?Component $owner = null): SshSession
    {
        $session = SshSession::pending();
        $owner?->__pamOwn($session);
        Native::call('connect', $options->payload(), static function (array $values) use ($session, $connected, $failed): void {
            $id = Native::string($values, 'id');
            if ($id === '') {
                $failed(new SshException('ssh: invalid session handle'));

                return;
            }
            if (!$session->opened($id, Native::string($values, 'fingerprint'), Native::string($values, 'algorithm'))) {
                return;
            }
            $connected($session);
        }, static function (SshException $error) use ($session, $failed): void {
            if ($session->disposed()) {
                return;
            }
            $session->dispose();
            $failed($error);
        });

        return $session;
    }
}

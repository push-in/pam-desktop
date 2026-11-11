<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Pam\Desktop\Modules\NativeModuleException;
use RuntimeException;
use Throwable;

/**
 * A failed SSH call. The message is a short code from the host
 * (`connect-failed: refused`, `sftp: no-such-file`, `exec-timeout`…); it never
 * contains the host, the user, a secret or a path.
 */
class SshException extends RuntimeException
{
    public function __construct(string $message, ?Throwable $previous = null)
    {
        parent::__construct($message, 0, $previous);
    }

    /** The typed exception for a failure reported by the `ssh` module. */
    public static function fromNative(NativeModuleException $error): self
    {
        $message = $error->getMessage();
        if (str_starts_with($message, 'host-key-mismatch:')) {
            return new HostKeyMismatch(substr($message, strlen('host-key-mismatch:')), $error);
        }
        if (str_starts_with($message, 'auth-failed')) {
            return new AuthFailed($message, $error);
        }

        return new self($message, $error);
    }
}

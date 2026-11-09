<?php

declare(strict_types=1);

namespace Pam\Desktop\Tooling;

use RuntimeException;

/**
 * Thrown by App::run() under `pam-desktop check` once the entry file has
 * registered its components: the application never boots.
 *
 * @internal
 */
final class ProjectCheckFinished extends RuntimeException
{
    public function __construct(public readonly mixed $root = null)
    {
        parent::__construct('pam-desktop check finished the entry file.');
    }
}

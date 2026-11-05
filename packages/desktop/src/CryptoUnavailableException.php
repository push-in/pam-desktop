<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;

/**
 * Neither the PHP extension nor the native host can perform a Pam\Desktop\Crypto
 * primitive (for example a PAM Desktop host older than 1.19.0 on a device).
 */
final class CryptoUnavailableException extends RuntimeException
{
}

<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;

/**
 * Neither the PHP extension nor the native host can perform a Pam\Desktop\Crypto
 * primitive (for example a PHP build without ext-sodium or ext-openssl).
 */
final class CryptoUnavailableException extends RuntimeException
{
}

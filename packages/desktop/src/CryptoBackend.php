<?php

declare(strict_types=1);

namespace Pam\Desktop;

/** Which implementation Pam\Desktop\Crypto uses for a primitive. */
enum CryptoBackend: int
{
    /** ext-sodium / ext-openssl in this PHP build. */
    case Extension = 1;
    /** A host function pam_desktop_crypto(). The desktop host does not define one. */
    case Native = 2;
    /** Neither: calls throw CryptoUnavailableException. */
    case Unavailable = 3;
}

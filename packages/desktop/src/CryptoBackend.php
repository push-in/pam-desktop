<?php

declare(strict_types=1);

namespace Pam\Desktop;

/** Which implementation Pam\Desktop\Crypto uses for a primitive. */
enum CryptoBackend: int
{
    /** ext-sodium / ext-openssl in this PHP build (desktop, server, tests). */
    case Extension = 1;
    /** The Android/iOS host through pam_desktop_crypto() (JCA, Kotlin, CryptoKit). */
    case Native = 2;
    /** Neither: calls throw CryptoUnavailableException. */
    case Unavailable = 3;
}

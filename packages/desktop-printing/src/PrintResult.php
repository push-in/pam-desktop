<?php

declare(strict_types=1);
namespace Pam\DesktopPrinting;
/** Accepted means the OS accepted the job, not that paper physically emerged. */
enum PrintResult: int { case Accepted = 1; case Cancelled = 2; }

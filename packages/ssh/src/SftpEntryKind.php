<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

enum SftpEntryKind: string
{
    case File = 'file';
    case Directory = 'dir';
    case Link = 'link';
    case Other = 'other';
}

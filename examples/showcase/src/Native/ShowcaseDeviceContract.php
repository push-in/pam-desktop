<?php

declare(strict_types=1);

namespace App\Native;

use Pam\Desktop\Bridge\Attributes\NativeMethod;
use Pam\Desktop\Bridge\Attributes\NativeModule;
use Pam\Desktop\Bridge\Attributes\NativePermission;
use Pam\Desktop\Bridge\NativeCallKind;

#[NativeModule(id: 1, name: 'ShowcaseDevice')]
interface ShowcaseDeviceContract
{
    /** @return array<string, mixed> */
    #[NativeMethod(id: 1)]
    #[NativePermission('device.info')]
    public function information(): array;

    /** @return array<string, mixed> */
    #[NativeMethod(id: 2, kind: NativeCallKind::Stream, timeoutMs: 60_000)]
    public function lifecycle(): array;
}

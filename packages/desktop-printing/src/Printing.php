<?php

declare(strict_types=1);
namespace Pam\DesktopPrinting;
use Closure;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModuleResult;
use Pam\Desktop\Modules\NativeModules;
use RuntimeException;
use Throwable;

final class Printing
{
    private static bool $pending = false;
    /** @param Closure(PrintResult):void $complete @param Closure(Throwable):void $failed */
    public static function print(Document $document, Closure $complete, Closure $failed): int
    {
        if (self::$pending) { $failed(new RuntimeException('A print dialog is already open.')); return 0; }
        $json = $document->json();
        self::$pending = true;
        try {
            return NativeModules::call('printing', 'print', ['document' => $json], static function (NativeModuleResult $result) use ($complete, $failed): void {
                self::$pending = false;
                if ($result->status === ModuleResultStatus::Failure) { $failed(new RuntimeException($result->payload)); return; }
                $values = Wire::decodeMap($result->payload);
                $value = $values['result'] ?? null;
                $status = is_int($value) ? PrintResult::tryFrom($value) : null;
                if ($status === null) { $failed(new RuntimeException('Invalid native print result.')); return; }
                $complete($status);
            });
        } catch (Throwable $error) { self::$pending = false; $failed($error); return 0; }
    }
}

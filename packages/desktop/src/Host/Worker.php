<?php

declare(strict_types=1);

namespace Pam\Desktop\Host;

use Pam\Desktop\Internal\Runtime;
use Throwable;

use function fwrite;
use function ini_set;
use function ob_start;
use function strlen;
use function substr;
use function unpack;

use const STDERR;

/**
 * Runs a PAM Desktop application inside the worker process: the entry script
 * boots the app (first frame), then this loop hands PHP one turn per batch of
 * host messages. PHP sleeps in a blocking read while the UI is idle.
 *
 * @internal
 */
final class Worker
{
    private function __construct()
    {
    }

    public static function run(string $entry): int
    {
        Channel::open();
        ini_set('display_errors', 'stderr');
        // stdout belongs to the channel: `echo`, `print` and `var_dump` go to the host log.
        ob_start(static function (string $chunk): string {
            if ($chunk !== '') {
                fwrite(STDERR, $chunk);
            }

            return '';
        }, 1);

        require_once __DIR__.'/functions.php';

        try {
            (static function (string $__pamEntry): void {
                require $__pamEntry;
            })($entry);
        } catch (Throwable $error) {
            self::report($error);
        }

        Runtime::deferRendering(true);
        self::turn();
        Channel::send(Channel::READY);
        Channel::flush();

        while (($message = Channel::read()) !== null) {
            [$type, $payload] = $message;
            try {
                switch ($type) {
                    case Channel::EVENT:
                        if (strlen($payload) >= 10) {
                            /** @var array{node: int, kind: int} $header */
                            $header = unpack('Pnode/vkind', $payload);
                            Runtime::dispatchEvent($header['node'], $header['kind'], substr($payload, 10));
                        }
                        break;
                    case Channel::MODULE_RESULT:
                        if (strlen($payload) >= 9) {
                            /** @var array{request: int, status: int} $header */
                            $header = unpack('Prequest/Cstatus', $payload);
                            Runtime::dispatchModuleResult($header['request'], $header['status'], substr($payload, 9));
                        }
                        break;
                    case Channel::FLUSH:
                        self::turn();
                        Channel::send(Channel::IDLE);
                        Channel::flush();
                        break;
                    case Channel::RESYNC:
                        Runtime::resynchronize();
                        break;
                    case Channel::CHECKPOINT:
                        Runtime::checkpointHotReload();
                        break;
                    case Channel::SHUTDOWN:
                        break 2;
                }
            } catch (Throwable $error) {
                self::report($error);
            }
        }

        try {
            Runtime::shutdown();
        } catch (Throwable) {
            // The process is leaving either way.
        }
        Channel::flush();

        return 0;
    }

    private static function turn(): void
    {
        try {
            Runtime::flush();
        } catch (Throwable $error) {
            self::report($error);
        }
    }

    private static function report(Throwable $error): void
    {
        try {
            Runtime::reportError($error);
        } catch (Throwable) {
            fwrite(STDERR, $error::class.': '.$error->getMessage()."\n");
        }
    }
}

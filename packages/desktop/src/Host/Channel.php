<?php

declare(strict_types=1);

namespace Pam\Desktop\Host;

use RuntimeException;

use function fflush;
use function fopen;
use function fread;
use function fwrite;
use function is_resource;
use function ord;
use function pack;
use function strlen;
use function substr;
use function unpack;

/**
 * Binary channel between the PHP worker process and the native desktop host.
 *
 * Every message is `u32 length (little-endian) · u8 type · payload`, where the
 * length counts the type byte. The worker owns stdout for this channel, so
 * application output is redirected to stderr before the entry script runs.
 *
 * @internal
 */
final class Channel
{
    /** Worker -> host. */
    public const int COMMIT = 0x01;
    public const int CALL = 0x02;
    public const int CALL_TYPED = 0x03;
    public const int ERROR = 0x04;
    public const int READY = 0x05;
    public const int IDLE = 0x06;
    public const int WINDOW = 0x07;

    /** Host -> worker. */
    public const int EVENT = 0x01;
    public const int MODULE_RESULT = 0x02;
    public const int FLUSH = 0x03;
    public const int RESYNC = 0x04;
    public const int SHUTDOWN = 0x05;
    public const int CHECKPOINT = 0x06;

    private const int MAX_MESSAGE_BYTES = 64 * 1024 * 1024;
    private const int FLUSH_THRESHOLD = 256 * 1024;

    /** @var resource|null */
    private static $input = null;
    /** @var resource|null */
    private static $output = null;
    private static string $pending = '';

    private function __construct()
    {
    }

    public static function open(): void
    {
        if (self::$input !== null) {
            return;
        }
        $input = fopen('php://stdin', 'rb');
        $output = fopen('php://stdout', 'wb');
        if (!is_resource($input) || !is_resource($output)) {
            throw new RuntimeException('The PAM Desktop host channel could not be opened.');
        }
        self::$input = $input;
        self::$output = $output;
    }

    public static function isOpen(): bool
    {
        return self::$output !== null;
    }

    public static function send(int $type, string $payload = ''): void
    {
        self::$pending .= pack('VC', strlen($payload) + 1, $type).$payload;
        if (strlen(self::$pending) >= self::FLUSH_THRESHOLD) {
            self::flush();
        }
    }

    public static function flush(): void
    {
        $output = self::$output;
        if ($output === null || self::$pending === '') {
            return;
        }
        $buffer = self::$pending;
        self::$pending = '';
        $length = strlen($buffer);
        $offset = 0;
        while ($offset < $length) {
            $written = @fwrite($output, $offset === 0 ? $buffer : substr($buffer, $offset));
            if ($written === false || $written === 0) {
                // The host is gone: nothing can be rendered any more.
                exit(0);
            }
            $offset += $written;
        }
        fflush($output);
    }

    /**
     * Blocks until the host sends a message; null when the host closed the channel.
     *
     * @return array{0: int, 1: string}|null
     */
    public static function read(): ?array
    {
        $header = self::readExactly(5);
        if ($header === null) {
            return null;
        }
        $fields = unpack('Vlength', $header);
        $length = $fields === false ? 0 : (int) $fields['length'];
        if ($length < 1 || $length > self::MAX_MESSAGE_BYTES) {
            throw new RuntimeException('The PAM Desktop host sent a malformed message.');
        }
        $payload = $length === 1 ? '' : self::readExactly($length - 1);
        if ($payload === null) {
            return null;
        }

        return [ord($header[4]), $payload];
    }

    private static function readExactly(int $length): ?string
    {
        $input = self::$input;
        if ($input === null) {
            return null;
        }
        $data = '';
        $remaining = $length;
        while ($remaining > 0) {
            $chunk = fread($input, $remaining);
            if ($chunk === false || $chunk === '') {
                return null;
            }
            $data .= $chunk;
            $remaining -= strlen($chunk);
        }

        return $data;
    }
}

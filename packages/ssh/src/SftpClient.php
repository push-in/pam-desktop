<?php

declare(strict_types=1);

namespace Pam\DesktopSsh;

use Closure;
use DomainException;
use JsonException;
use Pam\Desktop\Modules\WireBytes;

/**
 * SFTP on a session. Paths are remote paths; a leading `~` is the login
 * directory. Failures are `sftp: no-such-file`, `sftp: permission-denied`,
 * `sftp: failure`… without the server's text.
 *
 * Every method takes an optional failure closure; without it the failure is
 * thrown from the callback.
 */
final class SftpClient
{
    /** The largest read or write: what fits in one module message. */
    public const MAX_BYTES = 983_040;

    /** @internal */
    public function __construct(private readonly SshSession $session)
    {
    }

    /**
     * At most 5000 entries; `truncated` says when more exist.
     *
     * @param Closure(SftpListing): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function list(string $path, Closure $done, ?Closure $failed = null): void
    {
        $this->call('sftpList', ['path' => self::path($path)], static function (array $values) use ($done, $failed): void {
            try {
                $decoded = json_decode(Native::string($values, 'entries'), true, 4, JSON_THROW_ON_ERROR);
            } catch (JsonException $error) {
                $exception = new SshException('sftp: invalid listing', $error);
                $failed === null ? throw $exception : $failed($exception);

                return;
            }
            $entries = [];
            foreach (is_array($decoded) ? $decoded : [] as $entry) {
                if (is_array($entry)) {
                    $entries[] = SftpEntry::fromArray($entry);
                }
            }
            $done(new SftpListing($entries, Native::bool($values, 'truncated')));
        }, $failed);
    }

    /**
     * @param Closure(SftpEntry): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function stat(string $path, Closure $done, ?Closure $failed = null, bool $followLinks = true): void
    {
        $this->call('sftpStat', ['path' => self::path($path), 'noFollow' => !$followLinks], static function (array $values) use ($done): void {
            $done(SftpEntry::fromArray($values));
        }, $failed);
    }

    /**
     * @param Closure(): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function mkdir(string $path, Closure $done, ?Closure $failed = null, ?int $mode = null): void
    {
        $this->call('sftpMkdir', ['path' => self::path($path)] + self::mode($mode), static fn (array $values) => $done(), $failed);
    }

    /**
     * @param Closure(): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function rename(string $from, string $to, Closure $done, ?Closure $failed = null): void
    {
        $this->call('sftpRename', ['from' => self::path($from), 'to' => self::path($to)], static fn (array $values) => $done(), $failed);
    }

    /**
     * Removes a file, a link or an empty directory; with `$recursive`, a
     * directory and everything in it (links are removed, not followed).
     *
     * @param Closure(): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function remove(string $path, Closure $done, ?Closure $failed = null, bool $recursive = false): void
    {
        $this->call('sftpRemove', ['path' => self::path($path), 'recursive' => $recursive], static fn (array $values) => $done(), $failed);
    }

    /**
     * Reads up to `$maxBytes` from the start of a file (for previews and
     * editors; use download() for whole files).
     *
     * @param Closure(SftpReadResult): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function read(string $path, Closure $done, ?Closure $failed = null, int $maxBytes = self::MAX_BYTES): void
    {
        if ($maxBytes < 1 || $maxBytes > self::MAX_BYTES) {
            throw new DomainException('SFTP reads are 1 to 983040 bytes.');
        }
        $this->call('sftpRead', ['path' => self::path($path), 'maxBytes' => $maxBytes], static function (array $values) use ($done): void {
            $done(new SftpReadResult(Native::string($values, 'bytes'), Native::int($values, 'size'), Native::bool($values, 'truncated')));
        }, $failed);
    }

    /**
     * Creates or replaces a file with `$bytes` (at most 960 KiB; use upload()
     * for more).
     *
     * @param Closure(): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function write(string $path, string $bytes, Closure $done, ?Closure $failed = null, ?int $mode = null): void
    {
        if (strlen($bytes) > self::MAX_BYTES) {
            throw new DomainException('SFTP writes are at most 983040 bytes; use upload().');
        }
        $this->call('sftpWrite', ['path' => self::path($path), 'bytes' => new WireBytes($bytes)] + self::mode($mode), static fn (array $values) => $done(), $failed);
    }

    /**
     * The absolute form of a path; `~` gives the login directory.
     *
     * @param Closure(string): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function realpath(string $path, Closure $done, ?Closure $failed = null): void
    {
        $this->call('sftpRealpath', ['path' => self::path($path)], static function (array $values) use ($done): void {
            $done(Native::string($values, 'path'));
        }, $failed);
    }

    /**
     * Uploads a local file (an absolute path).
     *
     * @param (Closure(int $bytes, int $total): void)|null $progress
     * @param (Closure(int $bytes): void)|null $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function upload(string $localPath, string $remotePath, ?Closure $progress = null, ?Closure $done = null, ?Closure $failed = null, ?int $mode = null): Transfer
    {
        return $this->transfer('sftpUpload', ['localPath' => self::path($localPath), 'remotePath' => self::path($remotePath)] + self::mode($mode), $progress, $done, $failed);
    }

    /**
     * Downloads a remote file to a local absolute path.
     *
     * @param (Closure(int $bytes, int $total): void)|null $progress
     * @param (Closure(int $bytes): void)|null $done
     * @param (Closure(SshException): void)|null $failed
     */
    public function download(string $remotePath, string $localPath, ?Closure $progress = null, ?Closure $done = null, ?Closure $failed = null): Transfer
    {
        return $this->transfer('sftpDownload', ['remotePath' => self::path($remotePath), 'localPath' => self::path($localPath)], $progress, $done, $failed);
    }

    /**
     * @param array<string, string|int|bool> $payload
     * @param (Closure(int, int): void)|null $progress
     * @param (Closure(int): void)|null $done
     * @param (Closure(SshException): void)|null $failed
     */
    private function transfer(string $method, array $payload, ?Closure $progress, ?Closure $done, ?Closure $failed): Transfer
    {
        $transfer = new Transfer($progress, $done, $failed);
        $this->call($method, $payload, function (array $values) use ($transfer): void {
            $transfer->started(Native::string($values, 'transfer'));
            $this->session->track($transfer);
        }, static function (SshException $error) use ($transfer): void {
            $transfer->failed($error);
        });

        return $transfer;
    }

    /**
     * @param array<string, string|int|bool|WireBytes> $payload
     * @param Closure(array<string, string|int|float|bool>): void $done
     * @param (Closure(SshException): void)|null $failed
     */
    private function call(string $method, array $payload, Closure $done, ?Closure $failed): void
    {
        Native::call($method, ['id' => $this->session->handle()] + $payload, $done, $failed);
    }

    private static function path(string $path): string
    {
        if ($path === '' || strlen($path) > 4096 || preg_match('/[\x00-\x1f]/', $path) === 1) {
            throw new DomainException('SSH paths must have 1 to 4096 bytes and no control characters.');
        }

        return $path;
    }

    /** @return array<string, int> */
    private static function mode(?int $mode): array
    {
        if ($mode === null) {
            return [];
        }
        if ($mode < 0 || $mode > 0o7777) {
            throw new DomainException('A file mode is 0 to 0o7777.');
        }

        return ['mode' => $mode];
    }
}

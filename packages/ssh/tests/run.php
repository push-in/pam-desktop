<?php

declare(strict_types=1);

use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Modules\NativeModuleTransport;
use Pam\Desktop\Modules\WireBytes;
use Pam\DesktopSsh\AuthFailed;
use Pam\DesktopSsh\ExecResult;
use Pam\DesktopSsh\HostKeyMismatch;
use Pam\DesktopSsh\PortForward;
use Pam\DesktopSsh\SftpEntryKind;
use Pam\DesktopSsh\SftpListing;
use Pam\DesktopSsh\SftpReadResult;
use Pam\DesktopSsh\Shell;
use Pam\DesktopSsh\Ssh;
use Pam\DesktopSsh\SshEvent;
use Pam\DesktopSsh\SshEventKind;
use Pam\DesktopSsh\SshException;
use Pam\DesktopSsh\SshOptions;
use Pam\DesktopSsh\SshSession;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopSsh\\', dirname(__DIR__).'/src');
$loader->addPsr4('Pam\\Desktop\\', dirname(__DIR__, 2).'/desktop/src');

function sshCheck(bool $value, string $message): void
{
    if (!$value) {
        throw new RuntimeException($message);
    }
}

/** Records module calls; tests answer them in any order. */
final class SshTransport implements NativeModuleTransport
{
    /** @var list<array{string, string, array<string, string|int|float|bool>, Closure}> */
    public array $calls = [];

    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void
    {
        $this->calls[] = [$module, $method, Wire::decodeMap($payload), $complete];
    }

    /** @return array{string, string, array<string, string|int|float|bool>, Closure} */
    public function take(string $method): array
    {
        foreach ($this->calls as $index => $call) {
            if ($call[1] === $method) {
                array_splice($this->calls, $index, 1);

                return $call;
            }
        }
        throw new RuntimeException("No pending {$method} call.");
    }

    public function pending(string $method): bool
    {
        foreach ($this->calls as $call) {
            if ($call[1] === $method) {
                return true;
            }
        }

        return false;
    }

    /** @param array<string, string|int|float|bool|WireBytes> $values */
    public function succeed(string $method, array $values = []): array
    {
        $call = $this->take($method);
        ($call[3])(ModuleResultStatus::Success, $values === [] ? '' : Wire::map($values));

        return $call[2];
    }

    public function fail(string $method, string $message): array
    {
        $call = $this->take($method);
        ($call[3])(ModuleResultStatus::Failure, $message);

        return $call[2];
    }

    /** @param array<string, string|int|float|bool|WireBytes> $values */
    public function event(SshEventKind $kind, array $values = []): void
    {
        $this->succeed('next', ['kind' => $kind->value] + $values);
    }
}

$transport = new SshTransport();
NativeModules::useTransport($transport);

// Options: validation, payload, no secrets in dumps.
foreach ([
    static fn () => new SshOptions('', 'root', password: 'x'),
    static fn () => new SshOptions('bad host', 'root', password: 'x'),
    static fn () => new SshOptions('h', 'root'),
    static fn () => new SshOptions('h', 'root', 70000, password: 'x'),
    static fn () => new SshOptions('h', 'root', password: 'x', timeoutMs: 10),
] as $invalid) {
    try {
        $invalid();
        throw new LogicException('Accepted invalid SSH options.');
    } catch (DomainException) {
    }
}
$options = SshOptions::password('server.test', 'deploy', 's3cret', 2222, 'abc');
sshCheck($options->payload() === [
    'host' => 'server.test', 'port' => 2222, 'username' => 'deploy', 'timeoutMs' => 15000,
    'keepaliveSeconds' => 30, 'password' => 's3cret', 'expectedFingerprint' => 'abc',
], 'Options become the connect payload.');
sshCheck(!str_contains(print_r($options, true), 's3cret'), 'Dumps hide the password.');
sshCheck(!array_key_exists('password', SshOptions::key('h', 'u', 'KEY')->payload()), 'Unset secrets are not sent.');

// Failures become typed exceptions.
$error = null;
Ssh::connect($options, static fn () => throw new LogicException('connected'), static function (SshException $e) use (&$error): void { $error = $e; });
$transport->fail('connect', 'host-key-mismatch:NEWprint');
sshCheck($error instanceof HostKeyMismatch && $error->fingerprint === 'NEWprint', 'Host key mismatch carries the new fingerprint.');
Ssh::connect($options, static fn () => throw new LogicException('connected'), static function (SshException $e) use (&$error): void { $error = $e; });
$transport->fail('connect', 'auth-failed');
sshCheck($error instanceof AuthFailed, 'Authentication failure is typed.');
Ssh::connect($options, static fn () => throw new LogicException('connected'), static function (SshException $e) use (&$error): void { $error = $e; });
$transport->fail('connect', 'connect-failed: refused');
sshCheck($error::class === SshException::class && $error->getMessage() === 'connect-failed: refused', 'Other failures keep the code.');
sshCheck($transport->calls === [], 'A failed connect leaves no call behind.');

// Connect and pump events.
$connected = null;
$session = Ssh::connect($options, static function (SshSession $s) use (&$connected): void { $connected = $s; }, static fn (SshException $e) => throw $e);
sshCheck(!$session->isOpen(), 'Connecting is not open.');
$transport->succeed('connect', ['id' => 'S1', 'fingerprint' => 'FP', 'algorithm' => 'ssh-ed25519']);
sshCheck($connected === $session && $session->isOpen() && $session->fingerprint() === 'FP' && $session->algorithm() === 'ssh-ed25519', 'Connected session.');
sshCheck($transport->pending('next'), 'An open session waits for events.');
$events = [];
$session->onEvent(static function (SshEvent $event) use (&$events): void { $events[] = $event->kind; });

// exec with binary output.
$result = null;
$session->exec('uptime', static function (ExecResult $r) use (&$result): void { $result = $r; }, timeoutMs: 5000);
$payload = $transport->succeed('exec', ['exitCode' => 2, 'stdout' => new WireBytes("\xff\x00up"), 'stderr' => new WireBytes(''), 'truncated' => true]);
sshCheck($payload === ['command' => 'uptime', 'id' => 'S1', 'maxBytes' => 983040, 'timeoutMs' => 5000], 'exec payload.');
sshCheck($result instanceof ExecResult && $result->exitCode === 2 && $result->stdout === "\xff\x00up" && $result->truncated && !$result->succeeded(), 'exec result keeps raw bytes.');

// Shell: open, data, write, resize, exit.
$shell = null;
$session->shell(static function (Shell $s) use (&$shell): void { $shell = $s; }, cols: 120, rows: 40);
$payload = $transport->succeed('shellOpen', ['channel' => 'C1']);
sshCheck($payload['cols'] === 120 && $payload['rows'] === 40 && $payload['term'] === 'xterm-256color', 'shell.open payload.');
$output = '';
$exit = 'none';
$shell->onData(static function (string $bytes) use (&$output): void { $output .= $bytes; })->onExit(static function (?int $status) use (&$exit): void { $exit = $status; });
$transport->event(SshEventKind::ShellData, ['channel' => 'C1', 'bytes' => new WireBytes("\x1b[1m$ \xe2\x82")]);
sshCheck($output === "\x1b[1m$ \xe2\x82", 'Shell output arrives as raw bytes, even split UTF-8.');
sshCheck($transport->pending('next'), 'The pump re-arms after each event.');
$shell->write("ls\n\x03");
$payload = $transport->succeed('shellWrite');
sshCheck($payload === ['bytes' => "ls\n\x03", 'channel' => 'C1'], 'Keystrokes are sent as bytes.');
$shell->resize(100, 30);
sshCheck($transport->succeed('shellResize') === ['channel' => 'C1', 'cols' => 100, 'rows' => 30], 'Resize payload.');
$transport->event(SshEventKind::ShellExit, ['channel' => 'C1', 'exitCode' => 0]);
sshCheck($exit === 0 && !$shell->isOpen(), 'Shell exit.');

// SFTP.
$sftp = $session->sftp();
sshCheck($sftp === $session->sftp(), 'One SFTP client per session.');
$listing = null;
$sftp->list('~/logs', static function (SftpListing $l) use (&$listing): void { $listing = $l; });
$payload = $transport->succeed('sftpList', ['entries' => json_encode([
    ['name' => 'a.log', 'kind' => 'file', 'size' => 12, 'mtime' => 1700000000, 'permissions' => '0644', 'owner' => 'deploy', 'group' => 'deploy', 'uid' => 1000, 'gid' => 1000],
    ['name' => 'old', 'kind' => 'dir', 'size' => null, 'mtime' => null, 'permissions' => '0755', 'owner' => null, 'group' => null, 'uid' => null, 'gid' => null],
]), 'count' => 2, 'truncated' => false]);
sshCheck($payload === ['id' => 'S1', 'path' => '~/logs'], 'sftp.list payload.');
sshCheck(count($listing->entries) === 2 && $listing->entries[0]->size === 12 && $listing->entries[0]->owner === 'deploy'
    && $listing->entries[1]->kind === SftpEntryKind::Directory && $listing->entries[1]->isDirectory() && !$listing->truncated, 'Listing entries.');
$read = null;
$sftp->read('/etc/hosts', static function (SftpReadResult $r) use (&$read): void { $read = $r; }, maxBytes: 10);
$transport->succeed('sftpRead', ['bytes' => new WireBytes("\x00binary\xff"), 'size' => 99, 'truncated' => true]);
sshCheck($read->bytes === "\x00binary\xff" && $read->size === 99 && $read->truncated, 'Read result.');
$wrote = false;
$sftp->write('/tmp/x', "\x00\x01", static function () use (&$wrote): void { $wrote = true; }, mode: 0o600);
sshCheck($transport->succeed('sftpWrite') === ['bytes' => "\x00\x01", 'id' => 'S1', 'mode' => 0o600, 'path' => '/tmp/x'] && $wrote, 'Write sends bytes and mode.');
$failure = null;
$sftp->stat('/missing', static fn () => throw new LogicException('found'), static function (SshException $e) use (&$failure): void { $failure = $e; });
$transport->fail('sftpStat', 'sftp: no-such-file');
sshCheck($failure?->getMessage() === 'sftp: no-such-file', 'SFTP failures reach the failure closure.');
try {
    $sftp->read('/x', static fn () => null, maxBytes: 2_000_000);
    throw new LogicException('Accepted an oversized read.');
} catch (DomainException) {
}

// Transfers with progress.
$progress = [];
$done = null;
$upload = $sftp->upload('/home/me/big.iso', '~/big.iso', static function (int $bytes, int $total) use (&$progress): void { $progress[] = [$bytes, $total]; }, static function (int $bytes) use (&$done): void { $done = $bytes; });
sshCheck($upload->id() === '', 'A transfer has no id until the host accepts it.');
$transport->succeed('sftpUpload', ['transfer' => 'T1']);
sshCheck($upload->id() === 'T1', 'Transfer id.');
$transport->event(SshEventKind::TransferProgress, ['transfer' => 'T1', 'bytes' => 10, 'total' => 100]);
$transport->event(SshEventKind::TransferDone, ['transfer' => 'T1', 'bytes' => 100]);
sshCheck($progress === [[10, 100]] && $done === 100 && $upload->isFinished(), 'Progress then done.');
$download = $sftp->download('/var/log/syslog', '/tmp/syslog', failed: static function (SshException $e) use (&$failure): void { $failure = $e; });
$transport->succeed('sftpDownload', ['transfer' => 'T2']);
$download->cancel();
sshCheck($transport->succeed('sftpCancel') === ['transfer' => 'T2'], 'Cancel.');
$transport->event(SshEventKind::TransferFailed, ['transfer' => 'T2', 'message' => 'cancelled']);
sshCheck($failure->getMessage() === 'cancelled' && $download->isFinished(), 'Cancelled transfer fails with cancelled.');

// Port forward.
$forward = null;
$session->forward(0, 'db.internal', 5432, static function (PortForward $f) use (&$forward): void { $forward = $f; });
$payload = $transport->succeed('forwardLocal', ['forward' => 'F1', 'port' => 54321]);
sshCheck($payload['bindHost'] === '127.0.0.1' && $payload['bindPort'] === 0 && $payload['allowNonLoopback'] === false && $payload['remotePort'] === 5432, 'forward.local payload.');
sshCheck($forward->port() === 54321 && $forward->isActive(), 'Forward port.');
$transport->event(SshEventKind::ForwardOpened, ['forward' => 'F1', 'port' => 54321]);
$transport->event(SshEventKind::ForwardConnections, ['forward' => 'F1', 'connections' => 2, 'failures' => 1]);
sshCheck($forward->connections() === 2 && $forward->failures() === 1, 'Connection counts.');
$forward->stop();
sshCheck($transport->succeed('forwardStop') === ['forward' => 'F1'] && !$forward->isActive(), 'Stop forward.');

// The session fails: children end, listeners hear it, calls refuse.
$closedWith = 'none';
$session->shell(static function (Shell $s) use (&$shell): void { $shell = $s; });
$transport->succeed('shellOpen', ['channel' => 'C2']);
$session->onClose(static function (?SshException $e) use (&$closedWith): void { $closedWith = $e; });
$transport->event(SshEventKind::SessionFailed, ['message' => 'connection-lost']);
sshCheck($closedWith instanceof SshException && $closedWith->getMessage() === 'connection-lost', 'Session failure reaches onClose.');
sshCheck(!$shell->isOpen() && !$session->isOpen() && $session->disposed(), 'Children end with the session.');
sshCheck($transport->succeed('disconnect') === ['id' => 'S1'], 'The host handle is released.');
sshCheck(!$transport->pending('next'), 'No more reads after the end.');
sshCheck(in_array(SshEventKind::SessionFailed, $events, true) && in_array(SshEventKind::ShellData, $events, true), 'onEvent sees every event.');
try {
    $session->exec('id', static fn () => null);
    throw new LogicException('A closed session ran a command.');
} catch (DomainException) {
}

// Disconnecting while connecting releases the session once the host answers.
$late = Ssh::connect($options, static fn () => throw new LogicException('connected after dispose'), static fn () => null);
$late->dispose();
$transport->succeed('connect', ['id' => 'S2', 'fingerprint' => 'FP', 'algorithm' => 'ssh-ed25519']);
sshCheck($transport->succeed('disconnect') === ['id' => 'S2'] && !$transport->pending('next'), 'A session disposed while connecting is released.');

// An explicit disconnect ends quietly.
$quiet = Ssh::connect($options, static fn () => null, static fn () => null);
$transport->succeed('connect', ['id' => 'S3', 'fingerprint' => 'FP', 'algorithm' => 'ssh-ed25519']);
$quiet->onClose(static fn () => throw new LogicException('onClose after disconnect'));
$quiet->disconnect();
sshCheck($transport->succeed('disconnect') === ['id' => 'S3'], 'Disconnect.');
$transport->event(SshEventKind::SessionClosed);
sshCheck($transport->calls === [], 'Nothing is left pending.');

NativeModules::useTransport(null);
echo "SSH package: options, typed failures, events, exec, shell, SFTP, transfers, forwards and lifecycle passed.\n";

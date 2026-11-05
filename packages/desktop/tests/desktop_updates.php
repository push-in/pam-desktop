<?php

declare(strict_types=1);

use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModuleTransport;
use Pam\Desktop\UpdateState;
use Pam\Desktop\Updates;

$assert ??= static function (bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
};

/** A host that answers the `updates` module and delivers changes through the long poll. */
$updatesHost = new class implements NativeModuleTransport {
    /** @var list<string> */
    public array $calls = [];
    public ?Closure $waiter = null;
    public int $polls = 0;
    /** @var array<string, array<string, string|int>|string> method => snapshot, or a failure message */
    public array $replies = [];
    public bool $available = true;

    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void
    {
        if ($module !== 'updates' || !$this->available) {
            $complete(ModuleResultStatus::Failure, 'unknown module');

            return;
        }
        if ($method === 'next') {
            $this->polls++;
            $this->waiter = $complete;

            return;
        }
        $this->calls[] = $method;
        $reply = $this->replies[$method] ?? ['state' => 'idle', 'currentVersion' => '1.0.0'];
        is_string($reply)
            ? $complete(ModuleResultStatus::Failure, $reply)
            : $complete(ModuleResultStatus::Success, Wire::map($reply));
    }

    /** @param array<string, string|int> $snapshot */
    public function change(array $snapshot): void
    {
        $waiter = $this->waiter;
        $this->waiter = null;
        $waiter(ModuleResultStatus::Success, Wire::map($snapshot));
    }
};
Updates::resetRuntime();
Runtime::setModuleTransport($updatesHost);

// --- State --------------------------------------------------------------------
$updatesHost->replies['state'] = ['state' => 'idle', 'currentVersion' => '1.2.0', 'downloadedBytes' => 0, 'totalBytes' => 0, 'policy' => 'notify'];
$current = null;
Updates::state(static function (UpdateState $state) use (&$current): void { $current = $state; });
$assert(
    $current instanceof UpdateState && $current->status === UpdateState::IDLE && $current->currentVersion === '1.2.0'
        && $current->availableVersion === null && $current->notesUrl === null && $current->error === null && $current->policy === 'notify',
    'Updates::state reads the snapshot; absent values are null.',
);
$assert($current->isEnabled() && !$current->isBusy() && !$current->isAvailable() && $current->progress() === 0.0, 'An idle state is enabled and at rest.');
$assert($updatesHost->polls === 0, 'Reading the state does not start the long poll.');

$updatesHost->replies['state'] = ['state' => 'disabled', 'currentVersion' => ''];
Updates::state(static function (UpdateState $state) use (&$current): void { $current = $state; });
$assert(!$current->isEnabled() && $current->policy === null, 'A manifest without updates reports disabled.');

// --- Operations ---------------------------------------------------------------
$available = ['state' => 'available', 'currentVersion' => '1.2.0', 'availableVersion' => '1.3.0', 'downloadedBytes' => 0, 'totalBytes' => 4000, 'notesUrl' => 'https://example.com/releases/1.3.0', 'policy' => 'manual'];
$updatesHost->replies['check'] = $available;
$checked = null;
Updates::check(static function (UpdateState $state) use (&$checked): void { $checked = $state; });
$assert(
    $checked->isAvailable() && $checked->availableVersion === '1.3.0' && $checked->totalBytes === 4000 && $checked->notesUrl === 'https://example.com/releases/1.3.0',
    'Updates::check reports the available release.',
);
$updatesHost->replies['download'] = ['state' => 'ready', 'downloadedBytes' => 4000, 'totalBytes' => 4000] + $available;
$downloaded = null;
Updates::download(static function (UpdateState $state) use (&$downloaded): void { $downloaded = $state; });
$assert($downloaded->isReady() && $downloaded->progress() === 1.0, 'Updates::download reports the verified archive.');
$updatesHost->replies['install'] = ['state' => 'applying'] + $available;
$installing = null;
Updates::install(static function (UpdateState $state) use (&$installing): void { $installing = $state; });
$assert($installing->status === UpdateState::APPLYING && $installing->isBusy(), 'Updates::install reports that the application is being replaced.');
Updates::check();
Updates::download();
Updates::install();
$assert($updatesHost->calls === ['state', 'state', 'check', 'download', 'install', 'check', 'download', 'install'], 'Every call reaches the updates module; callbacks are optional.');

$updatesHost->replies['check'] = 'The update feed signature did not verify.';
$failed = null;
Updates::check(static function (UpdateState $state) use (&$failed): void { $failed = $state; });
$assert($failed->isFailed() && $failed->error === 'The update feed signature did not verify.', 'A refused operation completes with a failed state and its reason.');

// --- Changes ------------------------------------------------------------------
$seen = [];
$first = Updates::onChange(static function (UpdateState $state) use (&$seen): void { $seen[] = 'a:'.$state->status; });
$second = Updates::onChange(static function (UpdateState $state) use (&$seen): void { $seen[] = 'b:'.$state->status; });
$assert($first !== $second && $updatesHost->polls === 1 && $updatesHost->waiter !== null, 'Listeners share one long poll.');
$updatesHost->change(['state' => 'checking', 'currentVersion' => '1.2.0']);
$assert($seen === ['a:checking', 'b:checking'] && $updatesHost->polls === 2, 'A change reaches every listener and re-arms the poll.');
$progress = null;
$third = Updates::onChange(static function (UpdateState $state) use (&$progress): void { $progress = $state->progress(); });
$updatesHost->change(['state' => 'downloading', 'currentVersion' => '1.2.0', 'downloadedBytes' => 1000, 'totalBytes' => 4000]);
$assert($progress === 0.25 && $updatesHost->polls === 3, 'Download progress is a fraction of the signed size.');
Updates::offChange($first);
Updates::offChange($third);
$updatesHost->change(['state' => 'ready', 'currentVersion' => '1.2.0']);
$assert($seen === ['a:checking', 'b:checking', 'a:downloading', 'b:downloading', 'b:ready'], 'An unsubscribed listener is not called: '.json_encode($seen));

Updates::offChange($second);
$updatesHost->change(['state' => 'applying', 'currentVersion' => '1.2.0']);
$assert(count($seen) === 5 && $updatesHost->polls === 4 && $updatesHost->waiter === null, 'Without listeners the poll is not re-armed.');
$again = Updates::onChange(static function (UpdateState $state) use (&$seen): void { $seen[] = 'c:'.$state->status; });
$assert($updatesHost->polls === 5 && $updatesHost->waiter !== null, 'A new listener starts the poll again.');
$updatesHost->change(['state' => 'failed', 'currentVersion' => '1.2.0', 'error' => 'offline']);
$assert(end($seen) === 'c:failed', 'The restarted poll delivers changes.');
Updates::offChange($again);

// A malformed snapshot never breaks a listener.
$broken = null;
Updates::onChange(static function (UpdateState $state) use (&$broken): void { $broken = $state; });
$waiter = $updatesHost->waiter;
$updatesHost->waiter = null;
$waiter(ModuleResultStatus::Success, "\xff\xff");
$assert($broken instanceof UpdateState && $broken->status === UpdateState::DISABLED, 'A malformed snapshot decodes as disabled.');

// --- A host without the module ---------------------------------------------------
Updates::resetRuntime();
$updatesHost->available = false;
$polls = $updatesHost->polls;
$silent = Updates::onChange(static fn (UpdateState $state) => null);
$unavailable = null;
Updates::state(static function (UpdateState $state) use (&$unavailable): void { $unavailable = $state; });
$assert($unavailable->isFailed() && $unavailable->error === 'unknown module', 'Without the module every call fails with the reason.');
$updatesHost->available = true;
Updates::onChange(static fn (UpdateState $state) => null);
$assert($updatesHost->polls === $polls + 1, 'A failed poll stops; the next listener starts it again.');
Updates::offChange($silent);

// --- Value object -------------------------------------------------------------
$state = UpdateState::fromArray(['state' => 'downloading', 'currentVersion' => 7, 'downloadedBytes' => -5, 'totalBytes' => '12', 'error' => '']);
$assert($state->status === UpdateState::DOWNLOADING && $state->currentVersion === '' && $state->downloadedBytes === 0 && $state->totalBytes === 0 && $state->error === null, 'Unexpected value types fall back to defaults.');
$assert((new UpdateState(UpdateState::DOWNLOADING, downloadedBytes: 9, totalBytes: 3))->progress() === 1.0, 'Progress never exceeds 1.');
$assert((new ReflectionClass(UpdateState::class))->isReadOnly(), 'UpdateState is immutable.');

Runtime::setModuleTransport(null);
Updates::resetRuntime();

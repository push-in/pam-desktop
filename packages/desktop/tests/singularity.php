<?php

declare(strict_types=1);

use Pam\Desktop\Bridge\Attributes\NativeMethod;
use Pam\Desktop\Bridge\Attributes\NativeModule;
use Pam\Desktop\Bridge\Attributes\NativePermission;
use Pam\Desktop\Bridge\ContractCompiler;
use Pam\Desktop\Bridge\NativeCallKind;
use Pam\Desktop\Diagnostics\ReplayTimeline;
use Pam\Desktop\HotReload\StateSnapshot;
use Pam\Desktop\HotReload\HotReloadCoordinator;
use Pam\Desktop\InternalHttp\LocalTransport;
use Pam\Desktop\InternalHttp\Request;
use Pam\Desktop\InternalHttp\Response;
use Pam\Desktop\InternalHttp\Router;
use Pam\Desktop\LocalFirst\ConflictPolicy;
use Pam\Desktop\LocalFirst\EncryptedJournal;
use Pam\Desktop\LocalFirst\LocalRecord;
use Pam\Desktop\LocalFirst\LocalStore;
use Pam\Desktop\Plugin\PluginRegistry;
use Pam\Desktop\Plugin\RegistryEntry;
use Pam\Desktop\Plugin\TrustTier;
use Pam\Desktop\Scheduling\AsyncStream;
use Pam\Desktop\Scheduling\BackpressureException;
use Pam\Desktop\Scheduling\DeadlineExceededException;
use Pam\Desktop\Scheduling\TaskGroup;

#[NativeModule(id: 1, name: 'Camera')]
interface SingularityCameraContract
{
    #[NativeMethod(id: 1)]
    #[NativePermission('camera.capture')]
    public function capture(string $quality, bool $flash): array;

    #[NativeMethod(id: 2, kind: NativeCallKind::Stream, timeoutMs: 60_000)]
    public function frames(int $maximumFps): array;
}

$contracts = ContractCompiler::compile([SingularityCameraContract::class], 'Pam.Showcase');
$assert(
    $contracts->manifest['schema'] === 2
        && $contracts->manifest['modules'][0]['methods'][0]['permissions'] === ['camera.capture']
        && str_contains($contracts->kotlin, 'object CameraContract')
        && str_contains($contracts->swift, 'enum CameraContract')
        && strlen($contracts->fingerprint) === 64,
    'Singularity contracts must generate deterministic typed PHP, Kotlin and Swift bindings.',
);

$group = new TaskGroup(deadlineMs: 1_000);
$group->async(static fn (): int => 20);
$group->async(static fn (): int => 22);
$assert($group->awaitAll() === [20, 22], 'Structured task groups must await ordered child results.');

$late = new TaskGroup(deadlineMs: 1);
$late->async(static function (): void { usleep(20_000); });
$deadlineExceeded = false;
try {
    $late->awaitAll();
} catch (DeadlineExceededException) {
    $deadlineExceeded = true;
}
$assert($deadlineExceeded, 'Structured task groups must enforce one deadline across all children.');

$stream = new AsyncStream(2);
$stream->emit('first');
$stream->emit('second');
$backpressure = false;
try {
    $stream->emit('overflow');
} catch (BackpressureException) {
    $backpressure = true;
}
$assert($backpressure && $stream->next() === 'first', 'Async streams must apply bounded backpressure.');

$local = new LocalStore();
$local->put('todos', '1', ['title' => 'offline'], 100);
$local->merge([
    new LocalRecord('todos', '1', ['title' => 'server'], 2, 200),
], ConflictPolicy::LatestWriteWins);
$assert(
    $local->get('todos', '1')?->attributes['title'] === 'server' && count($local->drainOutbox()) === 1,
    'Local-first storage must support optimistic outbox writes and deterministic conflict resolution.',
);
$journal = new EncryptedJournal(str_repeat('k', 32));
$sealed = $journal->seal([['id' => 1, 'value' => 'secret']]);
$assert(
    !str_contains($sealed, 'secret') && $journal->open($sealed)[0]['value'] === 'secret',
    'Local-first journals must be authenticated and encrypted at rest.',
);

$router = (new Router())
    ->middleware(static function (Request $request, Closure $next): Response {
        $response = $next($request);
        return new Response($response->status, $response->body, $response->headers + ['x-pam-local' => '1']);
    })
    ->route('POST', '/process', static fn (Request $request): Response => Response::json([
        'length' => strlen($request->body),
    ]));
$transport = new LocalTransport($router);
$response = $transport->send(new Request('POST', '/process', body: 'payload'));
$assert(
    $response->status === 200 && $response->headers['x-pam-local'] === '1' && !$transport->opensNetworkSocket(),
    'Internal HTTP must run middleware and handlers without opening a TCP socket.',
);

$snapshot = StateSnapshot::capture(['route' => '/checkout', 'form' => ['email' => 'a@b.test']]);
$assert(
    $snapshot->restore()['route'] === '/checkout' && strlen($snapshot->fingerprint) === 64,
    'Hot reload must preserve bounded, fingerprinted application state.',
);
$reloadState = ['route' => '/home'];
$reload = new HotReloadCoordinator();
$reload->register(
    'navigation',
    static fn (): array => $reloadState,
    static function (array $state) use (&$reloadState): void { $reloadState = $state; },
);
$reload->checkpoint([['kind' => 1, 'route' => '/checkout']]);
$reloadState = ['route' => '/empty'];
$assert(
    $reload->restore() && $reloadState['route'] === '/home',
    'Hot reload coordinator must restore registered state across a runtime restart.',
);
$reload->clear();

$timeline = new ReplayTimeline();
$timeline->record(1, ['action' => 'tap']);
$timeline->record(2, ['route' => '/checkout']);
$replayed = [];
$timeline->replay(static function (int $kind, array $payload) use (&$replayed): void {
    $replayed[] = [$kind, $payload];
});
$assert(
    count($replayed) === 2 && strlen($timeline->fingerprint()) === 64,
    'DevTools replay must preserve deterministic event order and identity.',
);

$registry = new PluginRegistry();
$registry->register(new RegistryEntry(
    'pushinbr/pam-desktop-camera',
    '1.0.0',
    TrustTier::Official,
    ['camera.capture'],
    str_repeat('a', 64),
    100,
));
$assert(
    $registry->authorize(
        'pushinbr/pam-desktop-camera',
        ['camera.capture'],
        minimumTrust: TrustTier::Verified,
    )->trust === TrustTier::Official,
    'Plugin registry policy must enforce immutable identity, capabilities and quality score.',
);

<?php

declare(strict_types=1);

use Pam\Desktop\Database\SQLite;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Modules\NativeModuleTransport;
use Pam\Desktop\Sync\Durable\ClaimedOperation;
use Pam\Desktop\Sync\Durable\OutboxDispatcher;
use Pam\Desktop\Sync\Durable\OutboxDispatchReport;
use Pam\Desktop\Sync\Durable\OutboxResolution;
use Pam\Desktop\Sync\Durable\OutboxOperation;
use Pam\Desktop\Sync\Durable\OutboxState;
use Pam\Desktop\Sync\Durable\SQLiteOutbox;

$autoload = dirname(__DIR__).'/vendor/autoload.php';
require is_file($autoload) ? $autoload : dirname(__DIR__, 3).'/vendor/autoload.php';
spl_autoload_register(static function (string $class): void {
    $prefix = 'Pam\\Desktop\\Sync\\Durable\\';
    if (str_starts_with($class, $prefix)) require dirname(__DIR__).'/src/'.substr($class, strlen($prefix)).'.php';
});

enum OperationKind: int { case Sale = 1; case Refund = 2; }

function verify(bool $condition, string $message): void { if (!$condition) throw new RuntimeException($message); }
$directory = sys_get_temp_dir().'/pam-outbox-'.bin2hex(random_bytes(8));
mkdir($directory, 0700);
$transport = new class($directory) implements NativeModuleTransport {
    private mixed $process;
    private array $pipes = [];
    public function __construct(private readonly string $directory) { $this->start(); }
    private function start(): void
    {
        $environment = getenv();
        unset($environment['PHPRC'], $environment['PHP_INI_SCAN_DIR']);
        $this->process = proc_open([getenv('PAM_SYNC_SQLITE_TEST_PHP') ?: '/usr/bin/php', __DIR__.'/sqlite-worker.php', $this->directory], [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $this->pipes, env_vars: $environment);
        if (!is_resource($this->process)) throw new RuntimeException('Cannot start real SQLite test worker.');
    }
    public function restart(): void { $this->close(); $this->start(); }
    public function close(): void
    {
        foreach ($this->pipes as $pipe) fclose($pipe);
        proc_close($this->process);
    }
    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void
    {
        verify($module === 'sqlite', 'Only native SQLite requests are expected.');
        fwrite($this->pipes[0], json_encode(Wire::decodeMap($payload) + ['method' => $method], JSON_THROW_ON_ERROR)."\n");
        $line = fgets($this->pipes[1]);
        if ($line === false) throw new RuntimeException('SQLite worker failed: '.stream_get_contents($this->pipes[2]));
        $result = json_decode($line, true, flags: JSON_THROW_ON_ERROR);
        $response = isset($result['error']) ? $result['error'] : Wire::map(['rows' => json_encode($result['rows'], JSON_THROW_ON_ERROR), ...(isset($result['applied']) ? ['applied' => $result['applied']] : [])]);
        verify(strlen($response) < 1_048_512, 'Every test response fits the native result budget.');
        $complete(isset($result['error']) ? ModuleResultStatus::Failure : ModuleResultStatus::Success, $response);
    }
};
NativeModules::useTransport($transport);
$fail = static function (Throwable $error): never { throw $error; };
$complete = static function (): void {};
function rows(string $database, string $sql, array $arguments = []): array
{
    $rows = null;
    SQLite::query($database, $sql, $arguments, static function (array $value) use (&$rows): void { $rows = $value; }, static function (Throwable $error): never { throw $error; });
    return $rows ?? throw new RuntimeException('Expected completed SQL query.');
}
function claim(SQLiteOutbox $box, int $now): array
{
    $result = null;
    $box->claim($now, 100, 4, static function (array $value) use (&$result): void { $result = $value; }, static function (Throwable $error): never { throw $error; });
    return $result ?? throw new RuntimeException('Expected completed claim.');
}
try {
    $box = new SQLiteOutbox('account-1-store-1.sqlite');
    $box->initialize($complete, $fail);
    SQLite::execute($box->database, 'CREATE TABLE sales(id TEXT PRIMARY KEY, amount_cents INTEGER NOT NULL)', [], $complete, $fail);
    $sale = new OutboxOperation('sale-1', OperationKind::Sale, ['amount_cents' => 500, 'customer' => ['name' => 'José', 'id' => 8]], 1);
    $projection = [['sql' => 'INSERT INTO sales(id, amount_cents) VALUES (?, ?)', 'arguments' => ['sale-1', 500]]];
    $box->commit($sale, $projection, $complete, $fail);
    $box->commit(new OutboxOperation('sale-1', OperationKind::Sale, ['customer' => ['id' => 8, 'name' => 'José'], 'amount_cents' => 500], 2), $projection, $complete, $fail);
    verify(count(rows($box->database, 'SELECT * FROM sales')) === 1, 'Idempotent replay does not reapply the projection.');
    $collision = null;
    $box->commit(new OutboxOperation('sale-1', OperationKind::Refund, ['amount_cents' => 100], 1), [], $complete, static function (Throwable $error) use (&$collision): void { $collision = $error; });
    verify($collision instanceof InvalidArgumentException, 'Different content with the same id is rejected.');
    $failed = false;
    $box->commit(new OutboxOperation('rollback', OperationKind::Sale, [], 2), [['sql' => 'INSERT INTO missing_table VALUES (1)']], $complete, static function () use (&$failed): void { $failed = true; });
    verify($failed && rows($box->database, 'SELECT id FROM pam_durable_outbox WHERE id = ?', ['rollback']) === [], 'Projection failure rolls back its outbox record.');
    $first = claim($box, 10);
    verify(count($first) === 1 && $first[0]->attempts === 1 && claim($box, 11) === [], 'Live leases cannot be sent twice.');
    $transport->restart();
    verify(claim($box, 109) === [], 'Process restart preserves a live lease.');
    $second = claim($box, 110);
    verify(count($second) === 1 && $second[0]->attempts === 2 && $second[0]->lease !== $first[0]->lease, 'Expired in-flight operation recovers after process death.');
    $box->acknowledge($first[0], ['server_id' => 999], $complete, $fail);
    verify(rows($box->database, 'SELECT operation_state FROM pam_durable_outbox')[0]['operation_state'] === OutboxState::InFlight->value, 'Stale sender cannot acknowledge a new lease.');
    $box->retry($second[0], 500, 'offline', $complete, $fail);
    verify(claim($box, 499) === [], 'Backoff is persisted.');
    $third = claim($box, 500)[0];
    $box->acknowledge($third, ['server_id' => 42], $complete, $fail);
    $box->commit($sale, $projection, $complete, $fail);
    verify(claim($box, 1000) === [], 'Acknowledged operation stays idempotent on replay.');
    $box->commit(new OutboxOperation('reconcile', OperationKind::Sale, [], 1001), [], $complete, $fail);
    $old = claim($box, 1001)[0];
    $current = claim($box, 1101)[0];
    $applied = null;
    $setApplied = static function (bool $value) use (&$applied): void { $applied = $value; };
    $reconcile = [['sql' => 'UPDATE sales SET amount_cents = amount_cents + 100 WHERE id = ?', 'arguments' => ['sale-1']]];
    $box->acknowledgeWithProjection($old, [], $reconcile, $setApplied, $fail);
    verify($applied === false && rows($box->database, 'SELECT amount_cents FROM sales')[0]['amount_cents'] === 500, 'Stale lease applies no projection.');
    $failed = false;
    $box->acknowledgeWithProjection($current, [], [...$reconcile, ['sql' => 'INSERT INTO missing_table VALUES(1)']], $setApplied, static function () use (&$failed): void { $failed = true; });
    verify($failed && rows($box->database, 'SELECT amount_cents FROM sales')[0]['amount_cents'] === 500, 'Failed reconciliation rolls back all projection writes.');
    verify(rows($box->database, 'SELECT operation_state FROM pam_durable_outbox WHERE id = ?', ['reconcile'])[0]['operation_state'] === OutboxState::InFlight->value, 'Failed reconciliation preserves leased intent.');
    $box->acknowledgeWithProjection($current, ['server_id' => 99], $reconcile, $setApplied, $fail);
    verify($applied === true && rows($box->database, 'SELECT amount_cents FROM sales')[0]['amount_cents'] === 600, 'Current lease atomically applies projection and ack.');
    $box->acknowledgeWithProjection($current, [], $reconcile, $setApplied, $fail);
    verify($applied === false && rows($box->database, 'SELECT amount_cents FROM sales')[0]['amount_cents'] === 600, 'Repeated response cannot apply projection twice.');
    $dispatchBox = new SQLiteOutbox('dispatch-'.bin2hex(random_bytes(8)).'.sqlite');
    $dispatchBox->initialize($complete, $fail);
    SQLite::execute($dispatchBox->database, 'CREATE TABLE dispatch_projection(id TEXT PRIMARY KEY, remote_id INTEGER NOT NULL)', [], $complete, $fail);
    $dispatchBox->commit(new OutboxOperation('dispatch-success', OperationKind::Sale, ['amount_cents' => 900], 3000), [], $complete, $fail);
    $dispatcher = new OutboxDispatcher($dispatchBox);
    $dispatchReport = null;
    $dispatcher->dispatch(3000, static fn (): bool => true,
        static function (ClaimedOperation $operation, Closure $done): void {
            $done(OutboxResolution::applied(['remote_id' => 901], [[
                'sql' => 'INSERT INTO dispatch_projection(id, remote_id) VALUES (?, ?)',
                'arguments' => [$operation->identifier, 901],
            ]]));
        }, static function (OutboxDispatchReport $report) use (&$dispatchReport): void { $dispatchReport = $report; }, $fail);
    verify($dispatchReport?->claimed === 1 && $dispatchReport->applied === 1
        && rows($dispatchBox->database, 'SELECT remote_id FROM dispatch_projection WHERE id = ?', ['dispatch-success'])[0]['remote_id'] === 901,
        'Dispatcher acknowledges a validated response and applies its projection atomically: '.json_encode($dispatchReport));
    $dispatchBox->commit(new OutboxOperation('dispatch-retry', OperationKind::Sale, ['amount_cents' => 1000], 4000), [], $complete, $fail);
    $retryReport = null;
    $dispatcher->dispatch(4000, static fn (): bool => true,
        static function (ClaimedOperation $operation, Closure $done): void { $done(OutboxResolution::retry('Temporary network failure.')); },
        static function (OutboxDispatchReport $report) use (&$retryReport): void { $retryReport = $report; }, $fail);
    verify($retryReport?->retried === 1
        && rows($dispatchBox->database, 'SELECT available_at FROM pam_durable_outbox WHERE id = ?', ['dispatch-retry'])[0]['available_at'] === 5000,
        'Dispatcher persists bounded exponential retry delay.');
    $offlineCalls = 0;
    $dispatcher->dispatch(4999, static fn (): bool => false,
        static function () use (&$offlineCalls): void { ++$offlineCalls; }, static function (): void {}, $fail);
    verify($offlineCalls === 0, 'Offline dispatch does not claim or send operations.');
    $retryReport = null;
    $dispatcher->dispatch(5000, static fn (): bool => true,
        static function (ClaimedOperation $operation, Closure $done): void { $done(OutboxResolution::rejected('Invalid operation.')); },
        static function (OutboxDispatchReport $report) use (&$retryReport): void { $retryReport = $report; }, $fail);
    verify($retryReport?->rejected === 1
        && rows($dispatchBox->database, 'SELECT operation_state FROM pam_durable_outbox WHERE id = ?', ['dispatch-retry'])[0]['operation_state'] === OutboxState::Failed->value,
        'Permanent adapter failures remain visible in the durable outbox.');
    $other = new SQLiteOutbox('account-1-store-2.sqlite');
    $other->initialize($complete, $fail);
    verify(rows($other->database, 'SELECT id FROM pam_durable_outbox') === [], 'Store databases are isolated.');
    for ($i = 0; $i < 12; ++$i) $box->commit(new OutboxOperation('bulk-'.$i, OperationKind::Sale, ['receipt' => str_repeat('"', 30_000)], 2000 + $i), [], $complete, $fail);
    $batch = claim($box, 3000);
    verify(count($batch) === SQLiteOutbox::MAX_BATCH, 'Claim reads only the configured bounded batch.');
    $box->reject($batch[0], 'Invalid customer', $complete, $fail);
    verify(rows($box->database, 'SELECT operation_state FROM pam_durable_outbox WHERE id = ?', [$batch[0]->identifier])[0]['operation_state'] === OutboxState::Failed->value, 'Permanent errors remain durable.');
    verify(rows($box->database, 'SELECT count(*) AS total FROM pam_durable_outbox')[0]['total'] === 14, 'No pending intent is evicted by the batch or restart.');
    $namespaced = new SQLiteOutbox($box->database, 'wallet');
    $namespaced->initialize($complete, $fail);
    $namespaced->commit(new OutboxOperation('bulk-0', OperationKind::Sale, ['wallet' => true], 1), [], $complete, $fail);
    $walletClaims = claim($namespaced, 3000);
    verify(count($walletClaims) === 1 && $walletClaims[0]->payload === ['wallet' => true], 'Queue namespaces isolate identical keys and operation kinds in one database.');
    $namespaced->acknowledgeWithProjection($walletClaims[0], [], [], static function(bool $applied):void { verify($applied, 'Namespaced guarded acknowledgement works.'); }, $fail);
    verify(count(claim($namespaced, 4000)) === 0, 'Namespaced queue is acknowledged independently.');
    $invalidNamespace = false;
    try { new SQLiteOutbox($box->database, 'wallet;DROP TABLE'); } catch (InvalidArgumentException) { $invalidNamespace = true; }
    verify($invalidNamespace, 'Queue namespace cannot inject SQL identifiers.');
    $groups=new SQLiteOutbox('atomic-groups.sqlite');$groups->initialize($complete,$fail);
    SQLite::execute($groups->database,'CREATE TABLE entities(id INTEGER PRIMARY KEY,value TEXT NOT NULL)',[],$complete,$fail);
    $ops=[new OutboxOperation('parent',OperationKind::Sale,['name'=>'A'],1),new OutboxOperation('child',OperationKind::Refund,['parent'=>'parent'],2)];
    $projection=[['sql'=>'INSERT INTO entities(id,value) VALUES(1,?)','arguments'=>['A']]];
    $groups->commitBatch($ops,$projection,$complete,$fail);$groups->commitBatch($ops,$projection,$complete,$fail);
    verify(count(rows($groups->database,'SELECT * FROM entities'))===1&&count(rows($groups->database,'SELECT * FROM pam_durable_outbox'))===2,'Atomic group replay never reapplies projection.');
    $rolledBack=false;$groups->commitBatch([new OutboxOperation('other',OperationKind::Sale,[],3)],$projection,$complete,static function(Throwable $e)use(&$rolledBack):void{$rolledBack=true;});
    verify($rolledBack&&rows($groups->database,'SELECT * FROM pam_durable_outbox WHERE id=?',['other'])===[],'Projection failure rolls back every inserted group intent.');
    $conflict=false;$groups->commitBatch([new OutboxOperation('parent',OperationKind::Sale,['name'=>'changed'],1),$ops[1]],[],$complete,static function(Throwable $e)use(&$conflict):void{$conflict=true;});verify($conflict,'Changed group payload fails instead of treating partial match as replay.');
    echo "Durable outbox: real SQLite atomicity, idempotency, crash/lease recovery, stale responses, backoff, bounds and store isolation passed\n";
} finally {
    NativeModules::useTransport(null);
    $transport->close();
    foreach (glob($directory.'/*') ?: [] as $file) unlink($file);
    rmdir($directory);
}

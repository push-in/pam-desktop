# Background jobs

Pam Desktop 1.0 schedules periodic PHP work from typed application policy. Jobs
run through the same supervised worker as commands, so they retain PHP
application state, use the same crash recovery, and cannot execute concurrently
inside one Zend runtime.

With the Workstation background-agent profile, jobs run in a separate headless
OS process. The visible host waits for an explicit post-bootstrap readiness
handshake; spawn success alone is never reported as agent readiness. Agent boot
errors remain visible on stderr, and a bounded timeout terminates a stuck child.

## Declare a job

```php
use Pam\Desktop\BackgroundJob;
use Pam\Desktop\ClientEvent;
use Pam\Desktop\CommandResult;
use Pam\Desktop\JobContext;
use Pam\Desktop\JobOverlapPolicy;

$app->job(
    'heartbeat',
    BackgroundJob::every(30_000)
        ->initialDelay(5_000)
        ->timeout(3_000)
        ->overlap(JobOverlapPolicy::Skip)
        ->persistent(maximumAttempts: 4, retryBackoffMilliseconds: 500),
    static function (JobContext $job): CommandResult {
        return CommandResult::success([
            'runId' => $job->runId,
            'startedAtMs' => $job->startedAtMilliseconds,
        ])->event(new ClientEvent('heartbeat.updated', [
            'runId' => $job->runId,
        ]));
    },
);
```

Intervals range from one second to 24 hours. Initial delay ranges from zero to
24 hours, and timeout uses the normal 100 ms to 120 s command bounds.
`runOnStart()` is shorthand for an initial delay of zero.

Overlap policies are sequential integers:

| Value | Policy | Behavior |
| --- | --- | --- |
| `1` | Skip | skip this run when the PHP worker is busy |
| `2` | Wait | wait for the serialized worker, then execute |

The next interval begins after a run finishes. This avoids an unbounded queue
when a handler is slower than its interval.

## Lifecycle events

Every job publishes ordered frontend events:

- `pam.job.started` with `id`, `runId`, and `startedAtMs`;
- `pam.job.completed` with `id`, `runId`, and `result`;
- `pam.job.failed` with `id`, `runId`, integer error `code`, and `message`;
- `pam.job.skipped` with `id`, `runId`, and integer `reason`.

```js
window.pam.on("pam.job.completed", ({ id, result }) => {
    console.log(`${id} completed`, result);
});
```

Effects and application events returned by a successful handler are processed
before `pam.job.completed`. A timeout, cancellation, malformed response, or
worker crash invalidates the worker and prepares a new generation.

## Durable execution and crash recovery

`persistent()` writes an atomic journal beneath the operating-system user-data directory. State codes are sequential integers: pending `1` and running `2`. The journal records an execution as running before PHP receives it and records the next deadline only after the attempt sequence ends.

If the host, machine or PHP worker dies while the state is running, the next application start detects the unfinished state and executes it immediately. Failed attempts use bounded exponential backoff; `maximumAttempts` accepts 1–10 and backoff accepts 100 ms–24 hours. Non-persistent jobs retain the original at-most-once schedule and create no journal entries.

Handlers should be idempotent because recovery provides at-least-once delivery: a crash can happen after an external side effect but before the completion record reaches disk. Use a domain idempotency key when writing to remote systems.

The scheduler fsyncs and atomically renames every persistent state transition.
If the journal cannot be written, the affected job is stopped before execution
and emits `pam.job.journal-failed`; PAM never silently degrades a persistent job
into an in-memory one.

## Shutdown and hot reload

Each schedule uses an interruptible host wait. Stopping an in-process host or
hot-reloading PHP policy cancels active work, wakes sleeping schedules, joins
their threads, and only then installs the replacement schedule. This prevents
duplicate timers after development reloads and avoids waiting for a long
interval during shutdown.

With `Workstation::agent(background: true, ...)`, the schedule belongs to a
separate headless, per-application single-instance process. It owns a supervised
PHP worker and initializes the same capability-checked SQLite, search, process
and filesystem services without creating Servo or a native window. The UI host
therefore never runs a duplicate timer. Closing the graphical host leaves the
agent running; returning `ShellEffect::quit()` performs an explicit full
application quit and stops the agent through the secured local lifecycle
channel. Development policy reloads start or stop the agent as the profile
changes before replacing an in-process schedule.

# Background jobs

Pam Desktop 1.0 schedules periodic PHP work from typed application policy. Jobs
run through the same supervised worker as commands, so they retain PHP
application state, use the same crash recovery, and cannot execute concurrently
inside one Zend runtime.

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
        ->overlap(JobOverlapPolicy::Skip),
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
worker crash invalidates the worker and prepares a new generation for later
work. The failed job is never automatically replayed.

## Shutdown and hot reload

Each schedule uses an interruptible host wait. Closing the application or
hot-reloading PHP policy cancels active work, wakes sleeping schedules, joins
their threads, and only then installs the replacement schedule. This prevents
duplicate timers after development reloads and avoids waiting for a long
interval during shutdown.

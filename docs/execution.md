# Command execution lanes

The default PHP worker is stateful and deterministic. PAM Desktop can also
route explicitly marked commands through a supervised PHP worker pool so CPU
or blocking work does not hold the application's stateful command lane.

```php
use Pam\Desktop\CommandExecution;

$app
    ->parallelWorkers(4)
    ->command('project.save', $save, CommandExecution::Stateful)
    ->command('image.resize', $resize, CommandExecution::Parallel)
    ->command('report.export', $export, CommandExecution::Background);
```

The integer-backed execution contract is:

| Value | Case | Behavior |
| ---: | --- | --- |
| `1` | `Stateful` | Serialized through the primary long-lived worker |
| `2` | `Parallel` | Distributed round-robin through the lazy worker pool |
| `3` | `Background` | Uses the isolated pool and keeps promise/cancellation semantics |

No additional worker starts when every command is stateful. When a parallel or
background command exists, the host boots between one and sixteen independent
workers. Each worker has its own PHP memory: parallel handlers must derive
their state from their payload, a database or another explicit durable source.

Timeout and cancellation terminate the affected pool worker and prepare a
fresh generation for its next request. PAM never retries the interrupted
handler automatically because its side effects may already have happened.

Events, native shell callbacks, scheduled jobs and commands without explicit
execution metadata always remain on the primary stateful worker.

# Development event protocol

`pam-desktop dev` implements PAM development session event schema 1. Set
`PAM_DEV_EVENTS=1` (`json` and `jsonl` are aliases) to emit `@pam-event `
prefixed JSON Lines on standard error without changing the human interface.

Desktop uses `surfaceCode: 4`. It emits session start/readiness/stop and change,
reload start, success, or failure events. Asset reloads use `data.reloadCode: 1`;
PHP runtime reloads use `data.reloadCode: 2`.

The canonical cross-host envelope, integer event codes, compatibility rules,
and consumer guidance live in the PAM runtime documentation at
`docs/development-events.md`.

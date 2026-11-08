# PAM Desktop Sync

`pushinbr/pam-desktop-sync` provides a durable SQLite outbox and a bounded,
callback based dispatcher. It does not create a timer, choose an HTTP client,
or keep work running while an application is hidden. The application calls
`OutboxDispatcher::dispatch()` when it is visible and connected, and supplies
an adapter for its operation kinds.

The adapter validates server responses before returning
`OutboxResolution::applied()`. Its reconciliation projection must use fixed SQL
and bound values. Temporary transport failures use `OutboxResolution::retry()`;
permanent domain failures use `OutboxResolution::rejected()`. Applied state and
the local projection commit atomically against the current lease. Retries use
bounded exponential backoff, and stale leases cannot change local data.

The dispatcher processes at most four operations per call. Applications keep
their own operation enums, request contracts, projections, and connectivity or
visibility triggers; the outbox retains operations until they are applied or
explicitly rejected.

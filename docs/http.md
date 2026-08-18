# Native HTTP

Pam Desktop can perform bounded HTTPS requests outside the renderer. The
renderer never receives an unrestricted network primitive: every destination
must be named in the PHP capability manifest and every request stays inside
that exact origin and optional base path.

```php
use Pam\Desktop\Capabilities;
use Pam\Desktop\HttpOrigin;

$app->capabilities(
    Capabilities::none()->http(
        HttpOrigin::allow('api', 'https://api.example.com/v1'),
    ),
);
```

```js
const response = await window.pam.http.request("api", {
  method: 1, // HttpMethod::Get
  path: "/users/42",
  headers: { Accept: "application/json" },
  timeout: 10_000,
});

const user = JSON.parse(response.body);
```

`method` is an integer enum: `1` GET, `2` POST, `3` PUT, `4` PATCH, `5`
DELETE and `6` HEAD. Requests use the system-independent Rust TLS stack, do not
follow redirects, reject URL credentials, cookies and hop-by-hop headers, and
cap request and response bodies at 8 MiB and 16 MiB. Use the binary streaming
API for larger payloads. A base path such as `/v1` cannot be escaped by the
frontend.

`traceparent` and `tracestate` are also reserved to the host and rejected from
application-provided headers so renderer code cannot forge distributed trace
lineage. A future tracing option may inject a host-owned context after policy
validation; do not work around this boundary with a differently cased name.

The returned object contains `status`, `headers` and a UTF-8 `body`. HTTP
failures remain ordinary HTTP responses; capability, validation and transport
failures reject with a typed Pam error. Pass an `AbortSignal` to stop waiting
in JavaScript. A request already executing in the native blocking transport may
finish remotely after local cancellation, so mutating endpoints should use
idempotency keys.

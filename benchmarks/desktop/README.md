# Desktop benchmark fixture

Register a no-op command that returns `null`, include `bridge.js` in the local
frontend and run this in Servo's console or the application inspector:

```js
const result = await runPamBridgeBenchmark();
console.table(result.microseconds);
console.log(JSON.stringify(result, null, 2));
```

The harness performs 200 warm-up calls and 10,000 sequential measured calls,
then reports p50, p95, p99, throughput and the host diagnostics snapshot. Keep
the raw JSON as the CI artifact. Run release builds only and follow
[`docs/performance.md`](../../docs/performance.md) when comparing frameworks or
releases.

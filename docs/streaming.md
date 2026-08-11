# Binary streaming

The command protocol remains bounded JSON for control messages. Binary files
use a separate authenticated data plane, avoiding base64 expansion and the
one-megabyte worker envelope limit.

## Read with backpressure

```js
const { size, stream } = await pam.fs.openRead({
    root: "media",
    path: "video.mp4",
});

const reader = stream.getReader();
while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    await decoder.write(value);
}
```

The response is a native `ReadableStream<Uint8Array>`. Axum and Tokio pull
64-KiB chunks from the already-authorized capability file, so a slow consumer
does not force the complete file into Rust or JavaScript memory.

## Write a binary body

```js
await pam.fs.writeStream(
    { root: "exports", path: "archive.bin" },
    generatedReadableStream,
    { signal: abortController.signal },
);
```

`Blob`, `ArrayBuffer`, typed arrays and `ReadableStream` sources are accepted.
The gateway consumes request chunks with backpressure and reports the exact
`bytesWritten`. Cancellation closes the request and file. Like `writeText`, a
streaming write truncates its destination before writing; applications that
need transactional publication should stream to a temporary name and rename it
through a domain command after validation.

Both directions are limited to 4 GiB per operation. Targets retain the normal
named-root or opaque-grant rules, reject parent traversal and symbolic links,
and repeat origin, ephemeral-token and source-window validation.

# File watching

Every named filesystem root with read access can be watched without polling in
application JavaScript:

```js
const watch = await window.pam.fs.watch(
  "documents",
  { root: "data", path: "documents" },
);

const off = window.pam.on("pam.fs.changed", ({ watchId }) => {
  if (watchId === "documents") refreshDocuments();
});

await watch.close();
off();
```

Watch operation is an integer enum (`1` start, `2` stop). The host confines the
path to a named read-capable root, rejects grants and symlinks, snapshots on a
250 ms native worker cadence, coalesces bursts into `pam.fs.changed`, targets
events to the owning window and caps traversal at 10,000 regular files. Watchers
stop automatically with the gateway and never block the renderer or PHP worker.

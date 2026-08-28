# Background full-text search

PAM Desktop embeds SQLite FTS5 as a dedicated workstation index. The index is stored in the operating-system user-data directory, uses WAL, survives application restarts and never shares a connection with application databases.

```js
await pam.search.index("documents/readme.md", "README", markdown);
await pam.search.remove("documents/old.md");

const { rows } = await pam.search.query("native workstation", { limit: 50 });
// [{ path, title, excerpt, score }]
```

For a project-wide initial build, pass a named filesystem root that already has read permission:

```js
await pam.search.rebuild({ root: "project", path: "" });
```

Rebuild skips symbolic links, non-UTF-8 files and individual files larger than 2 MiB. Queries are capped at 4 KiB and 500 results. Results use SQLite BM25 ranking and bounded highlighted excerpts. Combine the native coalesced watcher with `index()` and `remove()` for incremental background maintenance; all gateway operations run on the blocking worker pool and never block rendering.

Operation codes are sequential integers: index `1`, remove `2`, query `3`, rebuild `4`, clear `5`. Direct indexing does not grant filesystem authority. Directory rebuild is accepted only after the host resolves the path inside an explicitly declared readable root.

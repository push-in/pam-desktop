use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::native::{FileTarget, NativeError};

const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const MAX_RESULTS: u16 = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum SearchOperation {
    Index = 1,
    Remove = 2,
    Query = 3,
    Rebuild = 4,
    Clear = 5,
}

impl TryFrom<u8> for SearchOperation {
    type Error = NativeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Index),
            2 => Ok(Self::Remove),
            3 => Ok(Self::Query),
            4 => Ok(Self::Rebuild),
            5 => Ok(Self::Clear),
            _ => Err(NativeError::invalid("Unknown search operation.")),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub window_id: String,
    pub operation: u8,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
    #[serde(default)]
    pub target: Option<FileTarget>,
}

pub struct SearchServices {
    connection: Mutex<Connection>,
}

impl SearchServices {
    pub fn prepare(project_root: &Path) -> Result<Self, String> {
        let path = index_path(project_root)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create search directory: {error}"))?;
        }
        let connection =
            Connection::open(path).map_err(|error| format!("cannot open search index: {error}"))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| format!("cannot enable search WAL: {error}"))?;
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS documents USING fts5(\
                 path UNINDEXED, title, content, tokenize='unicode61');",
            )
            .map_err(|error| format!("cannot prepare FTS5 index: {error}"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn dispatch(
        &self,
        request: &SearchRequest,
        authorized_path: Option<&Path>,
    ) -> Result<Value, NativeError> {
        let operation = SearchOperation::try_from(request.operation)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| NativeError::native("Search index lock is poisoned", request.operation))?;
        match operation {
            SearchOperation::Index => index_document(
                &mut connection,
                &request.path,
                &request.title,
                &request.content,
            ),
            SearchOperation::Remove => {
                let removed = connection
                    .execute("DELETE FROM documents WHERE path = ?1", [&request.path])
                    .map_err(search_error)?;
                Ok(json!({"removed": removed}))
            }
            SearchOperation::Query => search(&connection, &request.query, request.limit),
            SearchOperation::Rebuild => rebuild(
                &mut connection,
                authorized_path.ok_or_else(|| {
                    NativeError::invalid("Search rebuild requires an authorized filesystem target.")
                })?,
            ),
            SearchOperation::Clear => {
                let removed = connection
                    .execute("DELETE FROM documents", [])
                    .map_err(search_error)?;
                Ok(json!({"removed": removed}))
            }
        }
    }
}

fn index_document(
    connection: &mut Connection,
    path: &str,
    title: &str,
    content: &str,
) -> Result<Value, NativeError> {
    if path.is_empty() || path.len() > 4_096 || content.len() > MAX_CONTENT_BYTES {
        return Err(NativeError::invalid(
            "Indexed documents require a bounded path and at most 2 MiB of UTF-8 content.",
        ));
    }
    let transaction = connection.transaction().map_err(search_error)?;
    transaction
        .execute("DELETE FROM documents WHERE path = ?1", [path])
        .map_err(search_error)?;
    transaction
        .execute(
            "INSERT INTO documents(path,title,content) VALUES(?1,?2,?3)",
            params![path, title, content],
        )
        .map_err(search_error)?;
    transaction.commit().map_err(search_error)?;
    Ok(json!({"indexed": 1}))
}

fn search(connection: &Connection, value: &str, limit: u16) -> Result<Value, NativeError> {
    if value.trim().is_empty()
        || value.len() > MAX_QUERY_BYTES
        || !(1..=MAX_RESULTS).contains(&limit)
    {
        return Err(NativeError::invalid(
            "Search requires a 1–4096 byte query and a limit between 1 and 500.",
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT path,title,snippet(documents,2,'<mark>','</mark>','…',24),\
             bm25(documents) FROM documents WHERE documents MATCH ?1\
             ORDER BY bm25(documents) LIMIT ?2",
        )
        .map_err(search_error)?;
    let rows = statement
        .query_map(params![value, limit], |row| {
            Ok(json!({
                "path": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "excerpt": row.get::<_, String>(2)?,
                "score": row.get::<_, f64>(3)?,
            }))
        })
        .map_err(search_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(search_error)?;
    Ok(json!({"rows": rows}))
}

fn rebuild(connection: &mut Connection, root: &Path) -> Result<Value, NativeError> {
    let transaction = connection.transaction().map_err(search_error)?;
    transaction
        .execute("DELETE FROM documents", [])
        .map_err(search_error)?;
    let mut directories = vec![root.to_path_buf()];
    let mut indexed = 0_u64;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| NativeError::native("Cannot scan search root", error))?
        {
            let entry =
                entry.map_err(|error| NativeError::native("Cannot scan search entry", error))?;
            let kind = entry
                .file_type()
                .map_err(|error| NativeError::native("Cannot inspect search entry", error))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if !kind.is_file()
                || entry
                    .metadata()
                    .map_or(true, |value| value.len() > MAX_CONTENT_BYTES as u64)
            {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path().as_path())
                .to_string_lossy()
                .replace('\\', "/");
            transaction
                .execute(
                    "INSERT INTO documents(path,title,content) VALUES(?1,?2,?3)",
                    params![relative, entry.file_name().to_string_lossy(), content],
                )
                .map_err(search_error)?;
            indexed = indexed.saturating_add(1);
        }
    }
    transaction.commit().map_err(search_error)?;
    Ok(json!({"indexed": indexed}))
}

fn index_path(project_root: &Path) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .ok_or_else(|| "cannot locate the operating-system data directory".to_owned())?;
    let root = project_root
        .canonicalize()
        .map_err(|error| format!("cannot resolve search project root: {error}"))?;
    let digest = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
    Ok(base
        .join("pam-desktop/search")
        .join(&digest[..24])
        .join("index.sqlite"))
}

fn search_error(error: rusqlite::Error) -> NativeError {
    NativeError::native("Search index operation failed", error)
}

const fn default_limit() -> u16 {
    50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_ranks_and_replaces_documents() {
        let connection = Connection::open_in_memory().expect("memory database should open");
        connection.execute_batch(
            "CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED,title,content,tokenize='unicode61');",
        ).expect("FTS5 should be bundled");
        let service = SearchServices {
            connection: Mutex::new(connection),
        };
        let request = |path: &str, content: &str| SearchRequest {
            window_id: "main".to_owned(),
            operation: 1,
            path: path.to_owned(),
            title: path.to_owned(),
            content: content.to_owned(),
            query: String::new(),
            limit: 50,
            target: None,
        };
        service
            .dispatch(&request("one.txt", "native workstation search"), None)
            .expect("document should index");
        service
            .dispatch(&request("two.txt", "unrelated content"), None)
            .expect("second document should index");
        let result = service
            .dispatch(
                &SearchRequest {
                    window_id: "main".to_owned(),
                    operation: 3,
                    path: String::new(),
                    title: String::new(),
                    content: String::new(),
                    query: "workstation".to_owned(),
                    limit: 10,
                    target: None,
                },
                None,
            )
            .expect("query should run");
        assert_eq!(result["rows"].as_array().map(Vec::len), Some(1));
        assert_eq!(result["rows"][0]["path"], "one.txt");
        service
            .dispatch(&request("one.txt", "replacement"), None)
            .expect("same path should replace atomically");
        let empty = service
            .dispatch(
                &SearchRequest {
                    window_id: "main".to_owned(),
                    operation: 3,
                    path: String::new(),
                    title: String::new(),
                    content: String::new(),
                    query: "workstation".to_owned(),
                    limit: 10,
                    target: None,
                },
                None,
            )
            .expect("replacement query should run");
        assert_eq!(empty["rows"].as_array().map(Vec::len), Some(0));
    }
}

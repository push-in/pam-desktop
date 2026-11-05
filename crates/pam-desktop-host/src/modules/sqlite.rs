//! `sqlite`: the application's local databases.
//!
//! A database is a file under `<data dir>/pam-databases/`, opened on first
//! use and kept open for the life of the process. Every call runs on one
//! worker thread, so results arrive in call order and a statement never
//! races another. Parameters are bound with their JSON types; rows come back
//! as a JSON array of objects, bounded so that one result always fits the
//! bridge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pam_desktop_ui::modules::support::required_text;
use pam_desktop_ui::modules::{Module, Reply, SerialWorker, request_map};
use pam_desktop_ui::wiremap::WireMap;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, Statement, TransactionBehavior};

/// The directory of the databases inside the application's data directory.
pub const DATABASES_DIRECTORY: &str = "pam-databases";

const MAX_QUERY_ROWS: usize = 1000;
const MAX_QUERY_COLUMNS: usize = 256;
const MAX_BATCH: usize = 10_000;
const MAX_SQL_BYTES: usize = 1_048_576;
/// One mebibyte of bridge payload minus the `rows` envelope.
const MAX_RESULT_BYTES: usize = 1_048_576 - 64;

const SCALAR_ARGUMENTS: &str = "SQLite arguments must be scalar";
const ARGUMENT_SETS: &str = "SQLite executeMany requires between 1 and 10000 argument sets";
const FALLBACK: &str = "SQLite operation failed";

fn valid_database_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        // Plain file names only: these two are directories.
        && !matches!(name, "." | "..")
}

/// The message of a SQLite error, as SQLite words it.
fn message(error: &rusqlite::Error) -> String {
    let text = match error {
        rusqlite::Error::SqliteFailure(_, Some(text)) => text.clone(),
        other => other.to_string(),
    };
    if text.is_empty() {
        FALLBACK.into()
    } else {
        text
    }
}

fn too_large(bytes: usize) -> String {
    format!(
        "Native module value is too large: SQLite query result is {bytes} bytes \
         (limit {MAX_RESULT_BYTES}); page it with LIMIT/OFFSET or select fewer columns"
    )
}

// ---- parameters -----------------------------------------------------------

/// One positional parameter: `null`, a boolean (stored as 0 or 1), a number
/// (integer when it is one, real otherwise) or a string.
fn scalar(value: &serde_json::Value) -> Result<Value, String> {
    match value {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(flag) => Ok(Value::Integer(i64::from(*flag))),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(Value::Integer)
            .or_else(|| number.as_f64().map(Value::Real))
            .ok_or_else(|| SCALAR_ARGUMENTS.into()),
        serde_json::Value::String(text) => Ok(Value::Text(text.clone())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Err(SCALAR_ARGUMENTS.into()),
    }
}

/// The parameters of one execution: a JSON array of scalars.
fn arguments(value: &serde_json::Value) -> Result<Vec<Value>, String> {
    value
        .as_array()
        .ok_or(SCALAR_ARGUMENTS)?
        .iter()
        .map(scalar)
        .collect()
}

/// The parameters of a batch: between 1 and 10 000 arrays of scalars.
fn argument_sets(value: &serde_json::Value) -> Result<Vec<Vec<Value>>, String> {
    let sets = value
        .as_array()
        .filter(|sets| (1..=MAX_BATCH).contains(&sets.len()))
        .ok_or(ARGUMENT_SETS)?;
    sets.iter().map(arguments).collect()
}

fn parse(json: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(json).map_err(|_| String::from("SQLite arguments must be a JSON array"))
}

/// One statement of a `transaction` call with the parameters of each execution.
#[derive(Debug, PartialEq)]
struct Step {
    sql: String,
    sets: Vec<Vec<Value>>,
}

/// The statements of a `transaction` call: between 1 and 10 000 objects
/// `{sql, arguments}` or `{sql, argumentSets}`.
fn steps(value: &serde_json::Value) -> Result<Vec<Step>, String> {
    let statements = value
        .as_array()
        .filter(|statements| (1..=MAX_BATCH).contains(&statements.len()))
        .ok_or("SQLite transaction requires between 1 and 10000 statements")?;
    statements
        .iter()
        .map(|statement| {
            let sql = statement
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .filter(|sql| !sql.is_empty() && sql.len() <= MAX_SQL_BYTES)
                .ok_or("Invalid SQLite transaction SQL statement")?;
            let sets = match statement.get("argumentSets") {
                Some(sets) => argument_sets(sets)?,
                // A statement without `arguments` takes no parameters.
                None => vec![
                    statement
                        .get("arguments")
                        .map_or_else(|| Ok(Vec::new()), arguments)?,
                ],
            };
            Ok(Step {
                sql: sql.to_owned(),
                sets,
            })
        })
        .collect()
}

// ---- execution ------------------------------------------------------------

fn bind(statement: &mut Statement<'_>, arguments: &[Value]) -> rusqlite::Result<()> {
    statement.clear_bindings();
    for (index, value) in arguments.iter().enumerate() {
        statement.raw_bind_parameter(index + 1, value)?;
    }
    Ok(())
}

/// Runs `sql` once per set of parameters, discarding any rows it returns.
fn run(connection: &Connection, sql: &str, sets: &[Vec<Value>]) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(sql)?;
    for arguments in sets {
        bind(&mut statement, arguments)?;
        let mut rows = statement.raw_query();
        while rows.next()?.is_some() {}
    }
    Ok(())
}

/// Runs every step inside one transaction: all of them apply, or none.
fn run_atomically(connection: &mut Connection, steps: &[Step]) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for step in steps {
        run(&transaction, &step.sql, &step.sets)?;
    }
    transaction.commit()
}

/// Appends one column value as JSON; fails as soon as the result is known to
/// be too large, before the value is copied.
fn push_value(json: &mut String, value: ValueRef<'_>) -> Result<(), String> {
    let needed = match value {
        ValueRef::Text(text) => text.len(),
        ValueRef::Blob(bytes) => bytes.len().div_ceil(3).saturating_mul(4),
        ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => 0,
    };
    if json.len().saturating_add(needed) > MAX_RESULT_BYTES {
        return Err(too_large(json.len().saturating_add(needed)));
    }
    match value {
        ValueRef::Null => json.push_str("null"),
        ValueRef::Integer(number) => json.push_str(&number.to_string()),
        // JSON has no infinities: they read back as null.
        ValueRef::Real(number) => json.push_str(
            &serde_json::Number::from_f64(number)
                .map_or_else(|| String::from("null"), |number| number.to_string()),
        ),
        ValueRef::Text(text) => {
            json.push_str(&serde_json::Value::from(String::from_utf8_lossy(text)).to_string());
        }
        ValueRef::Blob(bytes) => {
            json.push('"');
            BASE64.encode_string(bytes, json);
            json.push('"');
        }
    }
    Ok(())
}

/// Runs a query and encodes its rows as a JSON array of objects keyed by
/// column name, in column order. Of two columns with one name, the last
/// gives the value.
fn query(connection: &Connection, sql: &str, arguments: &[Value]) -> Result<String, String> {
    let mut statement = connection.prepare(sql).map_err(|error| message(&error))?;
    let column_count = statement.column_count();
    let mut columns: Vec<(String, usize)> = Vec::new();
    for (index, name) in statement.column_names().into_iter().enumerate() {
        match columns.iter_mut().find(|(known, _)| known == name) {
            Some(column) => column.1 = index,
            None => columns.push((name.to_owned(), index)),
        }
    }
    let keys: Vec<(String, usize)> = columns
        .into_iter()
        .map(|(name, index)| (serde_json::Value::from(name).to_string(), index))
        .collect();
    bind(&mut statement, arguments).map_err(|error| message(&error))?;
    let mut rows = statement.raw_query();
    let mut json = String::from("[");
    let mut count = 0_usize;
    while let Some(row) = rows.next().map_err(|error| message(&error))? {
        if count >= MAX_QUERY_ROWS {
            return Err(
                "SQLite query exceeded the 1000-row bridge limit; paginate the query".into(),
            );
        }
        if column_count > MAX_QUERY_COLUMNS {
            return Err("SQLite query exceeded the 256-column bridge limit".into());
        }
        if count > 0 {
            json.push(',');
        }
        json.push('{');
        for (position, (key, index)) in keys.iter().enumerate() {
            if position > 0 {
                json.push(',');
            }
            json.push_str(key);
            json.push(':');
            let value = row.get_ref(*index).map_err(|error| message(&error))?;
            push_value(&mut json, value)?;
        }
        json.push('}');
        if json.len() > MAX_RESULT_BYTES {
            return Err(too_large(json.len()));
        }
        count += 1;
    }
    json.push(']');
    if json.len() > MAX_RESULT_BYTES {
        return Err(too_large(json.len()));
    }
    Ok(json)
}

// ---- databases ------------------------------------------------------------

/// The databases opened so far, by name.
struct Databases {
    directory: PathBuf,
    open: HashMap<String, Connection>,
}

impl Databases {
    /// The connection of `name`, opened and configured on first use.
    fn connection(&mut self, name: &str) -> Result<&mut Connection, String> {
        if !self.open.contains_key(name) {
            std::fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
            let connection =
                Connection::open(self.directory.join(name)).map_err(|error| message(&error))?;
            configure(&connection).map_err(|error| message(&error))?;
            self.open.insert(name.to_owned(), connection);
        }
        self.open.get_mut(name).ok_or_else(|| FALLBACK.into())
    }
}

fn configure(connection: &Connection) -> rusqlite::Result<()> {
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.busy_timeout(Duration::from_millis(5000))?;
    connection.pragma_update(None, "temp_store", "MEMORY")
}

/// Runs one call and encodes its result.
fn execute(
    databases: &Mutex<Databases>,
    method: &str,
    request: &WireMap,
) -> Result<Vec<u8>, String> {
    if !matches!(method, "execute" | "query" | "executeMany" | "transaction") {
        return Err(format!("Unknown SQLite method {method}"));
    }
    let name = required_text(request, "database")?;
    if !valid_database_name(name) {
        return Err("Invalid SQLite database name".into());
    }
    let sql = required_text(request, "sql")?;
    let parameters = parse(required_text(request, "arguments")?)?;
    if method != "transaction" && (sql.is_empty() || sql.len() > MAX_SQL_BYTES) {
        return Err("SQLite SQL must contain between 1 and 1048576 bytes".into());
    }
    let mut databases = databases.lock().map_err(|_| String::from(FALLBACK))?;
    let connection = databases.connection(name)?;
    match method {
        "execute" => {
            let arguments = arguments(&parameters)?;
            run(connection, sql, &[arguments]).map_err(|error| message(&error))?;
            Ok(Vec::new())
        }
        "query" => {
            let arguments = arguments(&parameters)?;
            let rows = query(connection, sql, &arguments)?;
            Ok(WireMap::new().with("rows", rows).encode())
        }
        "executeMany" => {
            let step = Step {
                sql: sql.to_owned(),
                sets: argument_sets(&parameters)?,
            };
            run_atomically(connection, &[step]).map_err(|error| message(&error))?;
            Ok(Vec::new())
        }
        _ => {
            let steps = steps(&parameters)?;
            run_atomically(connection, &steps).map_err(|error| message(&error))?;
            Ok(Vec::new())
        }
    }
}

pub struct Sqlite {
    databases: Arc<Mutex<Databases>>,
    worker: SerialWorker,
}

impl Sqlite {
    /// `data_dir` is the application's data directory; databases live in its
    /// `pam-databases` subdirectory.
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            databases: Arc::new(Mutex::new(Databases {
                directory: data_dir.join(DATABASES_DIRECTORY),
                open: HashMap::new(),
            })),
            worker: SerialWorker::new("pam-sqlite"),
        }
    }
}

impl Module for Sqlite {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        let databases = Arc::clone(&self.databases);
        let method = method.to_owned();
        self.worker
            .run(move || reply.result(execute(&databases, &method, &request)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;
    use pam_desktop_ui::runtime::{MODULE_FAILURE, MODULE_SUCCESS};
    use serde_json::json;

    const WAIT: Duration = Duration::from_secs(10);

    struct Fixture {
        directory: PathBuf,
        sqlite: Sqlite,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = temporary_directory(name);
            let sqlite = Sqlite::new(&directory);
            Self { directory, sqlite }
        }

        fn call(
            &mut self,
            method: &str,
            sql: &str,
            arguments: &serde_json::Value,
        ) -> Result<WireMap, String> {
            self.call_on("app.db", method, sql, arguments)
        }

        fn call_on(
            &mut self,
            database: &str,
            method: &str,
            sql: &str,
            arguments: &serde_json::Value,
        ) -> Result<WireMap, String> {
            let request = WireMap::new()
                .with("database", database)
                .with("sql", sql)
                .with("arguments", arguments.to_string());
            self.send(method, &request)
        }

        fn send(&mut self, method: &str, request: &WireMap) -> Result<WireMap, String> {
            let (reply, answer) = Reply::channel();
            self.sqlite.call(method, &request.encode(), reply);
            let (status, payload) = answer.recv_timeout(WAIT).unwrap();
            if status == MODULE_SUCCESS {
                Ok(request_map(&payload).unwrap())
            } else {
                assert_eq!(status, MODULE_FAILURE);
                Err(String::from_utf8(payload).unwrap())
            }
        }

        fn execute(&mut self, sql: &str, arguments: &serde_json::Value) {
            assert!(self.call("execute", sql, arguments).unwrap().is_empty());
        }

        fn rows(&mut self, sql: &str, arguments: &serde_json::Value) -> serde_json::Value {
            let result = self.call("query", sql, arguments).unwrap();
            serde_json::from_str(result.text("rows").unwrap()).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn parameters_keep_their_json_types() {
        assert_eq!(
            arguments(&json!([
                null,
                true,
                false,
                42,
                -7,
                1.5,
                "text",
                9_223_372_036_854_775_807_i64
            ]))
            .unwrap(),
            [
                Value::Null,
                Value::Integer(1),
                Value::Integer(0),
                Value::Integer(42),
                Value::Integer(-7),
                Value::Real(1.5),
                Value::Text("text".into()),
                Value::Integer(i64::MAX),
            ]
        );
        // Beyond a 64-bit integer the number is a real.
        assert_eq!(
            arguments(&json!([18_446_744_073_709_551_615_u64])).unwrap(),
            [Value::Real(18_446_744_073_709_551_615.0)]
        );
        assert_eq!(arguments(&json!([])).unwrap(), []);
        for invalid in [
            json!([[1]]),
            json!([{"a": 1}]),
            json!({"a": 1}),
            json!("x"),
            json!(null),
        ] {
            assert_eq!(arguments(&invalid).unwrap_err(), SCALAR_ARGUMENTS);
        }
    }

    #[test]
    fn batches_and_transactions_are_bounded() {
        assert_eq!(argument_sets(&json!([[1], [2]])).unwrap().len(), 2);
        assert_eq!(argument_sets(&json!([])).unwrap_err(), ARGUMENT_SETS);
        assert_eq!(argument_sets(&json!([1])).unwrap_err(), SCALAR_ARGUMENTS);
        assert_eq!(argument_sets(&json!({})).unwrap_err(), ARGUMENT_SETS);
        let limit = serde_json::Value::from(vec![json!([1]); MAX_BATCH]);
        assert_eq!(argument_sets(&limit).unwrap().len(), MAX_BATCH);
        let beyond = serde_json::Value::from(vec![json!([1]); MAX_BATCH + 1]);
        assert_eq!(argument_sets(&beyond).unwrap_err(), ARGUMENT_SETS);

        assert_eq!(
            steps(&json!([
                {"sql": "DELETE FROM t", "arguments": []},
                {"sql": "INSERT INTO t VALUES (?)", "arguments": [], "argumentSets": [[1], [2]]},
                {"sql": "VACUUM"},
            ]))
            .unwrap(),
            [
                Step {
                    sql: "DELETE FROM t".into(),
                    sets: vec![vec![]]
                },
                Step {
                    sql: "INSERT INTO t VALUES (?)".into(),
                    sets: vec![vec![Value::Integer(1)], vec![Value::Integer(2)]]
                },
                Step {
                    sql: "VACUUM".into(),
                    sets: vec![vec![]]
                },
            ]
        );
        const COUNT: &str = "SQLite transaction requires between 1 and 10000 statements";
        assert_eq!(steps(&json!([])).unwrap_err(), COUNT);
        assert_eq!(steps(&json!({})).unwrap_err(), COUNT);
        let beyond = serde_json::Value::from(vec![json!({"sql": "SELECT 1"}); MAX_BATCH + 1]);
        assert_eq!(steps(&beyond).unwrap_err(), COUNT);
        const STATEMENT: &str = "Invalid SQLite transaction SQL statement";
        assert_eq!(steps(&json!([{"sql": ""}])).unwrap_err(), STATEMENT);
        assert_eq!(steps(&json!([{"arguments": []}])).unwrap_err(), STATEMENT);
        assert_eq!(steps(&json!([{"sql": 5}])).unwrap_err(), STATEMENT);
        assert_eq!(steps(&json!(["SELECT 1"])).unwrap_err(), STATEMENT);
        assert_eq!(
            steps(&json!([{"sql": "x".repeat(MAX_SQL_BYTES + 1)}])).unwrap_err(),
            STATEMENT
        );
        assert_eq!(
            steps(&json!([{"sql": "SELECT ?", "arguments": [[1]]}])).unwrap_err(),
            SCALAR_ARGUMENTS
        );
        assert_eq!(
            steps(&json!([{"sql": "SELECT ?", "argumentSets": []}])).unwrap_err(),
            ARGUMENT_SETS
        );
    }

    #[test]
    fn database_names_are_plain_file_names() {
        assert!(valid_database_name("app.db"));
        assert!(valid_database_name("cache_v2-test.sqlite"));
        assert!(valid_database_name(&"n".repeat(128)));
        for invalid in [
            "",
            ".",
            "..",
            "a/b.db",
            "../x.db",
            "a\\b",
            "file:x?mode=memory",
            "a b",
            "ação.db",
        ] {
            assert!(!valid_database_name(invalid), "{invalid}");
        }
        assert!(!valid_database_name(&"n".repeat(129)));
    }

    #[test]
    fn rows_are_encoded_by_column_type_in_column_order() {
        let mut fixture = Fixture::new("sqlite-rows");
        fixture.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price REAL, data BLOB, note TEXT)",
            &json!([]),
        );
        fixture.execute(
            "INSERT INTO items (id, name, price, data, note) VALUES (?, ?, ?, x'00ff10', ?)",
            &json!([7, "Café \"quente\"\n", 12.5, null]),
        );
        fixture.execute(
            "INSERT INTO items (id, name, price) VALUES (?, ?, ?)",
            &json!([9_007_199_254_740_993_i64, true, 3]),
        );
        let result = fixture
            .call(
                "query",
                "SELECT note, data, price, name, id FROM items ORDER BY id",
                &json!([]),
            )
            .unwrap();
        // The text itself keeps the column order of the query. A boolean is
        // bound as 1, which the TEXT column stores as text.
        assert_eq!(
            result.text("rows").unwrap(),
            "[{\"note\":null,\"data\":\"AP8Q\",\"price\":12.5,\"name\":\"Café \\\"quente\\\"\\n\",\"id\":7},\
             {\"note\":null,\"data\":null,\"price\":3.0,\"name\":\"1\",\"id\":9007199254740993}]"
        );
        // Bound types select rows as SQLite compares them.
        assert_eq!(
            fixture.rows(
                "SELECT id FROM items WHERE price > ? AND name = ?",
                &json!([10, "Café \"quente\"\n"])
            ),
            json!([{"id": 7}])
        );
        assert_eq!(
            fixture.rows("SELECT id FROM items WHERE id = ?", &json!(["none"])),
            json!([])
        );
        assert_eq!(
            fixture.rows(
                "SELECT typeof(?) AS a, typeof(?) AS b, typeof(?) AS c, typeof(?) AS d",
                &json!([1, 1.5, "x", null])
            ),
            json!([{"a": "integer", "b": "real", "c": "text", "d": "null"}])
        );
        // Of two columns with one name, the last gives the value.
        assert_eq!(
            fixture
                .call("query", "SELECT 1 AS n, 2 AS other, 3 AS n", &json!([]))
                .unwrap()
                .text("rows"),
            Some("[{\"n\":3,\"other\":2}]")
        );
        assert_eq!(
            fixture.rows("SELECT 9e999 AS huge, -0.5 AS half", &json!([])),
            json!([{"huge": null, "half": -0.5}])
        );
    }

    #[test]
    fn databases_are_opened_lazily_with_the_expected_pragmas() {
        let mut fixture = Fixture::new("sqlite-pragmas");
        assert!(!fixture.directory.join(DATABASES_DIRECTORY).exists());
        assert_eq!(
            fixture.rows("PRAGMA journal_mode", &json!([])),
            json!([{"journal_mode": "wal"}])
        );
        assert_eq!(
            fixture.rows("PRAGMA foreign_keys", &json!([])),
            json!([{"foreign_keys": 1}])
        );
        assert_eq!(
            fixture.rows("PRAGMA synchronous", &json!([])),
            json!([{"synchronous": 1}])
        );
        assert_eq!(
            fixture.rows("PRAGMA busy_timeout", &json!([])),
            json!([{"timeout": 5000}])
        );
        assert_eq!(
            fixture.rows("PRAGMA temp_store", &json!([])),
            json!([{"temp_store": 2}])
        );
        assert!(
            fixture
                .directory
                .join(DATABASES_DIRECTORY)
                .join("app.db")
                .is_file()
        );

        // Databases are independent files; a connection keeps its state between calls.
        fixture.execute("CREATE TABLE t (v)", &json!([]));
        fixture
            .call_on("other.db", "execute", "CREATE TABLE u (v)", &json!([]))
            .unwrap();
        assert!(
            fixture
                .call_on("other.db", "query", "SELECT * FROM t", &json!([]))
                .unwrap_err()
                .contains("no such table")
        );
        fixture.execute("CREATE TEMP TABLE scratch (v)", &json!([]));
        fixture.execute("INSERT INTO scratch VALUES (1)", &json!([]));
        assert_eq!(
            fixture.rows("SELECT v FROM scratch", &json!([])),
            json!([{"v": 1}])
        );
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let mut fixture = Fixture::new("sqlite-foreign-keys");
        fixture.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)", &json!([]));
        fixture.execute(
            "CREATE TABLE child (parent INTEGER REFERENCES parent(id))",
            &json!([]),
        );
        let error = fixture
            .call("execute", "INSERT INTO child VALUES (?)", &json!([1]))
            .unwrap_err();
        assert_eq!(error, "FOREIGN KEY constraint failed");
    }

    #[test]
    fn execute_many_is_atomic() {
        let mut fixture = Fixture::new("sqlite-batch");
        fixture.execute(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            &json!([]),
        );
        let inserted = fixture.call(
            "executeMany",
            "INSERT INTO t (id, name) VALUES (?, ?)",
            &json!([[1, "a"], [2, "b"], [3, "c"]]),
        );
        assert!(inserted.unwrap().is_empty());
        assert_eq!(
            fixture.rows("SELECT count(*) AS n FROM t", &json!([])),
            json!([{"n": 3}])
        );
        // The third set violates the primary key: nothing of the batch stays.
        let error = fixture
            .call(
                "executeMany",
                "INSERT INTO t (id, name) VALUES (?, ?)",
                &json!([[10, "x"], [11, "y"], [1, "duplicate"]]),
            )
            .unwrap_err();
        assert_eq!(error, "UNIQUE constraint failed: t.id");
        assert_eq!(
            fixture.rows("SELECT count(*) AS n FROM t", &json!([])),
            json!([{"n": 3}])
        );
        assert_eq!(
            fixture
                .call(
                    "executeMany",
                    "INSERT INTO t (id, name) VALUES (?, ?)",
                    &json!([])
                )
                .unwrap_err(),
            ARGUMENT_SETS
        );
        // The connection is usable after a rolled back batch.
        fixture.execute("INSERT INTO t (id, name) VALUES (?, ?)", &json!([4, "d"]));
    }

    #[test]
    fn transactions_apply_entirely_or_not_at_all() {
        let mut fixture = Fixture::new("sqlite-transaction");
        fixture.execute(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            &json!([]),
        );
        let statements = json!([
            {"sql": "INSERT INTO t (id, name) VALUES (?, ?)", "arguments": [1, "one"]},
            {"sql": "INSERT INTO t (id, name) VALUES (?, ?)", "arguments": [], "argumentSets": [[2, "two"], [3, "three"]]},
            {"sql": "UPDATE t SET name = upper(name) WHERE id = ?", "arguments": [1]},
        ]);
        assert!(
            fixture
                .call("transaction", "", &statements)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fixture.rows("SELECT id, name FROM t ORDER BY id", &json!([])),
            json!([{"id": 1, "name": "ONE"}, {"id": 2, "name": "two"}, {"id": 3, "name": "three"}])
        );
        let failing = json!([
            {"sql": "DELETE FROM t", "arguments": []},
            {"sql": "INSERT INTO t (id, name) VALUES (?, ?)", "arguments": [9, null]},
        ]);
        let error = fixture.call("transaction", "", &failing).unwrap_err();
        assert_eq!(error, "NOT NULL constraint failed: t.name");
        assert_eq!(
            fixture.rows("SELECT count(*) AS n FROM t", &json!([])),
            json!([{"n": 3}])
        );
        assert_eq!(
            fixture.call("transaction", "", &json!([])).unwrap_err(),
            "SQLite transaction requires between 1 and 10000 statements"
        );
        assert_eq!(
            fixture
                .call("transaction", "", &json!([{"sql": ""}]))
                .unwrap_err(),
            "Invalid SQLite transaction SQL statement"
        );
    }

    #[test]
    fn results_are_bounded_for_the_bridge() {
        let mut fixture = Fixture::new("sqlite-limits");
        fixture.execute("CREATE TABLE n (v INTEGER)", &json!([]));
        let values: Vec<serde_json::Value> = (0..1001).map(|value| json!([value])).collect();
        fixture
            .call(
                "executeMany",
                "INSERT INTO n VALUES (?)",
                &serde_json::Value::from(values),
            )
            .unwrap();
        assert_eq!(
            fixture
                .rows("SELECT v FROM n LIMIT 1000", &json!([]))
                .as_array()
                .unwrap()
                .len(),
            1000
        );
        assert_eq!(
            fixture
                .call("query", "SELECT v FROM n", &json!([]))
                .unwrap_err(),
            "SQLite query exceeded the 1000-row bridge limit; paginate the query"
        );

        let wide = |count: usize| {
            let columns: Vec<String> = (0..count)
                .map(|index| format!("{index} AS c{index}"))
                .collect();
            format!("SELECT {}", columns.join(", "))
        };
        assert_eq!(
            fixture.rows(&wide(256), &json!([]))[0]
                .as_object()
                .unwrap()
                .len(),
            256
        );
        assert_eq!(
            fixture.call("query", &wide(257), &json!([])).unwrap_err(),
            "SQLite query exceeded the 256-column bridge limit"
        );

        // Large values fail before they are copied into the result.
        let error = fixture
            .call("query", "SELECT zeroblob(800000) AS data", &json!([]))
            .unwrap_err();
        assert_eq!(
            error,
            "Native module value is too large: SQLite query result is 1066677 bytes (limit 1048512); \
             page it with LIMIT/OFFSET or select fewer columns"
        );
        let error = fixture
            .call(
                "query",
                "SELECT v, printf('%.*c', 2000, 'x') AS pad FROM n LIMIT 600",
                &json!([]),
            )
            .unwrap_err();
        assert!(
            error.starts_with("Native module value is too large: SQLite query result is "),
            "{error}"
        );
        let fits = fixture
            .call("query", "SELECT zeroblob(700000) AS data", &json!([]))
            .unwrap();
        assert!(fits.text("rows").unwrap().len() <= MAX_RESULT_BYTES);
    }

    #[test]
    fn invalid_calls_fail_with_the_sdk_messages() {
        let mut fixture = Fixture::new("sqlite-failures");
        assert_eq!(
            fixture.call("vacuum", "VACUUM", &json!([])).unwrap_err(),
            "Unknown SQLite method vacuum"
        );
        assert_eq!(
            fixture.send("query", &WireMap::new()).unwrap_err(),
            "Missing text field database"
        );
        assert_eq!(
            fixture
                .send("query", &WireMap::new().with("database", "app.db"))
                .unwrap_err(),
            "Missing text field sql"
        );
        assert_eq!(
            fixture
                .send(
                    "query",
                    &WireMap::new()
                        .with("database", "app.db")
                        .with("sql", "SELECT 1")
                )
                .unwrap_err(),
            "Missing text field arguments"
        );
        assert_eq!(
            fixture
                .call_on("../escape.db", "query", "SELECT 1", &json!([]))
                .unwrap_err(),
            "Invalid SQLite database name"
        );
        assert!(!fixture.directory.join("escape.db").exists());
        assert_eq!(
            fixture.call("query", "", &json!([])).unwrap_err(),
            "SQLite SQL must contain between 1 and 1048576 bytes"
        );
        assert_eq!(
            fixture
                .call("query", "SELECT ?", &json!([[1, 2]]))
                .unwrap_err(),
            SCALAR_ARGUMENTS
        );
        assert!(
            fixture
                .call("execute", "SELEC 1", &json!([]))
                .unwrap_err()
                .contains("syntax error")
        );
        assert!(
            fixture
                .call("query", "SELECT * FROM missing", &json!([]))
                .unwrap_err()
                .contains("no such table: missing")
        );
        // More parameters than placeholders is an error, fewer bind NULL.
        assert!(
            fixture
                .call("query", "SELECT ? AS v", &json!([1, 2]))
                .is_err()
        );
        assert_eq!(
            fixture.rows("SELECT ? AS a, ? AS b", &json!([1])),
            json!([{"a": 1, "b": null}])
        );
        // A failure does not poison the worker.
        assert_eq!(
            fixture.rows("SELECT 1 AS one", &json!([])),
            json!([{"one": 1}])
        );
    }
}

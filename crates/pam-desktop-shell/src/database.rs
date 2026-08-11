use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use pam_desktop_protocol::{DatabaseAccess, DatabaseConfig, DatabaseOperation, ErrorCode};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::native::NativeError;

const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_PARAMETERS: usize = 1_024;
const MAX_ROWS: usize = 10_000;
const MAX_COLUMNS: usize = 256;
const MAX_TRANSACTIONS: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseRequest {
    pub window_id: String,
    pub database: String,
    pub operation: DatabaseOperation,
    #[serde(default)]
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<Value>,
    #[serde(default)]
    pub statements: Vec<DatabaseStatement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseStatement {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<Value>,
}

struct DatabaseHandle {
    access: DatabaseAccess,
    connection: Mutex<Connection>,
}

pub struct DatabaseServices {
    handles: HashMap<String, DatabaseHandle>,
}

impl DatabaseServices {
    pub fn prepare(project_root: &Path, configs: &[DatabaseConfig]) -> Result<Self, String> {
        let project_root = std::fs::canonicalize(project_root)
            .map_err(|error| format!("cannot resolve the database project root: {error}"))?;
        let mut handles = HashMap::with_capacity(configs.len());
        for config in configs {
            config.validate()?;
            let relative = safe_relative_path(&config.path)?;
            let path = project_root.join(relative);
            if let Some(parent) = path.parent() {
                if config.access == DatabaseAccess::ReadWrite {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        format!(
                            "cannot create database directory {}: {error}",
                            parent.display()
                        )
                    })?;
                }
                let canonical_parent = std::fs::canonicalize(parent).map_err(|error| {
                    format!(
                        "cannot resolve database directory {}: {error}",
                        parent.display()
                    )
                })?;
                if !canonical_parent.starts_with(&project_root) {
                    return Err(format!(
                        "database {:?} escapes the project root",
                        config.name
                    ));
                }
            }
            if path.is_symlink() {
                return Err(format!(
                    "database {:?} cannot be a symbolic link",
                    config.name
                ));
            }
            let flags = match config.access {
                DatabaseAccess::Read => OpenFlags::SQLITE_OPEN_READ_ONLY,
                DatabaseAccess::ReadWrite => {
                    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
                }
            } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let connection = Connection::open_with_flags(&path, flags).map_err(|error| {
                format!(
                    "cannot open database {:?} at {}: {error}",
                    config.name,
                    path.display()
                )
            })?;
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .map_err(|error| format!("cannot configure database {:?}: {error}", config.name))?;
            if config.access == DatabaseAccess::ReadWrite {
                connection
                    .pragma_update(None, "journal_mode", "WAL")
                    .map_err(|error| format!("cannot enable WAL for {:?}: {error}", config.name))?;
                connection
                    .pragma_update(None, "foreign_keys", true)
                    .map_err(|error| {
                        format!("cannot enable foreign keys for {:?}: {error}", config.name)
                    })?;
            }
            handles.insert(
                config.name.clone(),
                DatabaseHandle {
                    access: config.access,
                    connection: Mutex::new(connection),
                },
            );
        }
        Ok(Self { handles })
    }

    pub fn dispatch(&self, request: &DatabaseRequest) -> Result<Value, NativeError> {
        let handle = self
            .handles
            .get(&request.database)
            .ok_or_else(|| NativeError::disabled(format!("database {:?}", request.database)))?;
        let mut connection = handle
            .connection
            .lock()
            .map_err(|_| NativeError::native("Cannot lock the database", "lock is poisoned"))?;
        match request.operation {
            DatabaseOperation::Query => query(&connection, &request.sql, &request.parameters),
            DatabaseOperation::Execute => {
                require_write(handle.access)?;
                execute(&connection, &request.sql, &request.parameters)
            }
            DatabaseOperation::Transaction => {
                require_write(handle.access)?;
                if request.statements.is_empty() || request.statements.len() > MAX_TRANSACTIONS {
                    return Err(NativeError::invalid(format!(
                        "Transactions require 1 to {MAX_TRANSACTIONS} statements."
                    )));
                }
                let transaction = connection.transaction().map_err(database_error)?;
                let mut affected = 0_u64;
                for statement in &request.statements {
                    affected = affected.saturating_add(
                        execute(&transaction, &statement.sql, &statement.parameters)?
                            .get("changes")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
                }
                transaction.commit().map_err(database_error)?;
                Ok(json!({"changes": affected}))
            }
        }
    }
}

fn query(connection: &Connection, sql: &str, parameters: &[Value]) -> Result<Value, NativeError> {
    validate_statement(sql, parameters)?;
    let values = sql_parameters(parameters)?;
    let mut statement = connection.prepare(sql).map_err(database_error)?;
    if statement.column_count() > MAX_COLUMNS {
        return Err(NativeError::too_large(format!(
            "Database results are limited to {MAX_COLUMNS} columns."
        )));
    }
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut cursor = statement
        .query(params_from_iter(values.iter()))
        .map_err(database_error)?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next().map_err(database_error)? {
        if rows.len() == MAX_ROWS {
            return Err(NativeError::too_large(format!(
                "Database queries are limited to {MAX_ROWS} rows."
            )));
        }
        let mut object = Map::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            object.insert(
                name.clone(),
                json_value(row.get_ref(index).map_err(database_error)?)?,
            );
        }
        rows.push(Value::Object(object));
    }
    Ok(json!({"rows": rows}))
}

fn execute(connection: &Connection, sql: &str, parameters: &[Value]) -> Result<Value, NativeError> {
    validate_statement(sql, parameters)?;
    let values = sql_parameters(parameters)?;
    let changes = connection
        .execute(sql, params_from_iter(values.iter()))
        .map_err(database_error)?;
    Ok(json!({
        "changes": changes,
        "lastInsertRowId": connection.last_insert_rowid(),
    }))
}

fn validate_statement(sql: &str, parameters: &[Value]) -> Result<(), NativeError> {
    if sql.trim().is_empty() || sql.len() > MAX_SQL_BYTES {
        return Err(NativeError::invalid(format!(
            "SQL statements must contain 1 to {MAX_SQL_BYTES} bytes."
        )));
    }
    if parameters.len() > MAX_PARAMETERS {
        return Err(NativeError::too_large(format!(
            "SQL statements accept at most {MAX_PARAMETERS} parameters."
        )));
    }
    Ok(())
}

fn sql_parameters(parameters: &[Value]) -> Result<Vec<SqlValue>, NativeError> {
    parameters
        .iter()
        .map(|value| match value {
            Value::Null => Ok(SqlValue::Null),
            Value::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
            Value::Number(value) if value.is_i64() => Ok(SqlValue::Integer(
                value.as_i64().expect("the number was checked as i64"),
            )),
            Value::Number(value) if value.is_u64() => {
                i64::try_from(value.as_u64().expect("the number was checked as u64"))
                    .map(SqlValue::Integer)
                    .map_err(|_| NativeError::invalid("Unsigned SQL parameters must fit in i64."))
            }
            Value::Number(value) => value.as_f64().map(SqlValue::Real).ok_or_else(|| {
                NativeError::invalid("The SQL parameter is not a finite JSON number.")
            }),
            Value::String(value) => Ok(SqlValue::Text(value.clone())),
            Value::Array(_) | Value::Object(_) => Err(NativeError::invalid(
                "SQL parameters must be null, booleans, numbers, or strings.",
            )),
        })
        .collect()
}

fn json_value(value: ValueRef<'_>) -> Result<Value, NativeError> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(value) => Ok(json!(value)),
        ValueRef::Real(value) => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| NativeError::native("Cannot encode database value", "non-finite real")),
        ValueRef::Text(value) => std::str::from_utf8(value)
            .map(|value| Value::String(value.to_owned()))
            .map_err(|_| NativeError::invalid("Database text results must be valid UTF-8.")),
        ValueRef::Blob(_) => Err(NativeError::invalid(
            "Blob columns require the streaming API and cannot use query().",
        )),
    }
}

fn safe_relative_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("database paths must be project-relative without parent components".to_owned());
    }
    Ok(path.to_path_buf())
}

fn require_write(access: DatabaseAccess) -> Result<(), NativeError> {
    if access == DatabaseAccess::ReadWrite {
        Ok(())
    } else {
        Err(NativeError::permission("The database is read-only."))
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "used directly as rusqlite Result::map_err adapter"
)]
fn database_error(error: rusqlite::Error) -> NativeError {
    NativeError {
        code: ErrorCode::NativeOperationFailed,
        message: format!("Database operation failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_protocol::{DatabaseAccess, DatabaseOperation};

    #[test]
    fn executes_queries_and_atomic_transactions() {
        let root = std::env::temp_dir().join(format!(
            "pam-desktop-database-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the system clock should follow the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("the test root should be created");
        let service = DatabaseServices::prepare(
            &root,
            &[DatabaseConfig {
                name: "app".to_owned(),
                path: "data/app.sqlite".to_owned(),
                access: DatabaseAccess::ReadWrite,
            }],
        )
        .expect("the database should open");

        service
            .dispatch(&request(
                DatabaseOperation::Execute,
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
                Vec::new(),
            ))
            .expect("the schema should be created");
        service
            .dispatch(&DatabaseRequest {
                window_id: "main".to_owned(),
                database: "app".to_owned(),
                operation: DatabaseOperation::Transaction,
                sql: String::new(),
                parameters: Vec::new(),
                statements: vec![
                    DatabaseStatement {
                        sql: "INSERT INTO notes (body) VALUES (?)".to_owned(),
                        parameters: vec![json!("one")],
                    },
                    DatabaseStatement {
                        sql: "INSERT INTO notes (body) VALUES (?)".to_owned(),
                        parameters: vec![json!("two")],
                    },
                ],
            })
            .expect("the transaction should commit");
        let result = service
            .dispatch(&request(
                DatabaseOperation::Query,
                "SELECT id, body FROM notes ORDER BY id",
                Vec::new(),
            ))
            .expect("the query should succeed");
        assert_eq!(result["rows"].as_array().map(Vec::len), Some(2));
        assert_eq!(result["rows"][1]["body"], "two");

        std::fs::remove_dir_all(&root).expect("the isolated test root should be removable");
    }

    #[test]
    fn rejects_database_path_escape() {
        assert!(safe_relative_path("../outside.sqlite").is_err());
        assert!(safe_relative_path("/tmp/outside.sqlite").is_err());
    }

    fn request(operation: DatabaseOperation, sql: &str, parameters: Vec<Value>) -> DatabaseRequest {
        DatabaseRequest {
            window_id: "main".to_owned(),
            database: "app".to_owned(),
            operation,
            sql: sql.to_owned(),
            parameters,
            statements: Vec::new(),
        }
    }
}

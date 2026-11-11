//! SFTP over one subsystem channel per session: listings, metadata, small
//! reads and writes, and uploads and downloads with progress events.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::modules::support::uuid;
use pam_desktop_ui::wiremap::{WireMap, WireValue};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::events::Event;
use super::session::Connection;
use super::{MAX_BYTES_VALUE, MAX_PATH, MAX_TRANSFERS, Permit, Shared, input, lock};

/// Entries in one listing.
pub const MAX_ENTRIES: usize = 5000;
/// Size of a listing's JSON.
const MAX_LISTING_BYTES: usize = 900 * 1024;
/// Bytes per SFTP read or write request.
const CHUNK: u32 = 32 * 1024;
/// Requests in flight during a transfer.
const PIPELINE: usize = 16;
/// Progress events per second and transfer.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Entries a recursive remove visits.
const MAX_REMOVE: usize = 100_000;
const MAX_DEPTH: usize = 64;

pub struct Transfer {
    pub session: String,
    cancel: watch::Sender<bool>,
}

impl Transfer {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
}

fn status_name(code: StatusCode) -> String {
    let name = format!("{code:?}");
    let mut out = String::with_capacity(name.len() + 4);
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A failure message that never carries the server's text (it may name
/// paths): `sftp: no-such-file`, `sftp: permission-denied`…
fn sftp_error(error: &SftpError) -> String {
    match error {
        SftpError::Status(status) => format!("sftp: {}", status_name(status.status_code)),
        SftpError::Timeout => "sftp: timeout".into(),
        SftpError::Limited(_) => "sftp: server limit".into(),
        SftpError::IO(_) => "sftp: connection lost".into(),
        _ => "sftp: protocol error".into(),
    }
}

fn kind(attributes: &FileAttributes) -> &'static str {
    use russh_sftp::protocol::FileType;
    match attributes.file_type() {
        FileType::Dir => "dir",
        FileType::File => "file",
        FileType::Symlink => "link",
        FileType::Other => "other",
    }
}

fn permissions(attributes: &FileAttributes) -> Option<String> {
    attributes
        .permissions
        .map(|mode| format!("{:04o}", mode & 0o7777))
}

/// `ls -l` columns 3 and 4, for servers (SFTP v3) that send no names.
fn owner_group(longname: &str) -> (Option<&str>, Option<&str>) {
    let mut columns = longname.split_whitespace();
    let owner = columns.nth(2);
    let group = columns.next();
    (owner, group)
}

fn entry_json(name: &str, longname: &str, attributes: &FileAttributes) -> serde_json::Value {
    let (owner, group) = owner_group(longname);
    let owner = attributes.user.as_deref().or(owner);
    let group = attributes.group.as_deref().or(group);
    json!({
        "name": name,
        "kind": kind(attributes),
        "size": attributes.size,
        "mtime": attributes.mtime,
        "permissions": permissions(attributes),
        "owner": owner,
        "group": group,
        "uid": attributes.uid,
        "gid": attributes.gid,
    })
}

fn insert_attributes(map: &mut WireMap, attributes: &FileAttributes) {
    map.insert("kind", kind(attributes));
    if let Some(size) = attributes.size {
        map.insert("size", i64::try_from(size).unwrap_or(i64::MAX));
    }
    if let Some(mtime) = attributes.mtime {
        map.insert("mtime", i64::from(mtime));
    }
    if let Some(permissions) = permissions(attributes) {
        map.insert("permissions", permissions);
    }
    if let Some(uid) = attributes.uid {
        map.insert("uid", i64::from(uid));
    }
    if let Some(gid) = attributes.gid {
        map.insert("gid", i64::from(gid));
    }
}

fn mode(data: &WireMap) -> Result<Option<u32>, String> {
    match data.get("mode") {
        None => Ok(None),
        Some(_) => {
            let mode = input::integer(data, "mode", 0, 0, 0o7777)?;
            Ok(u32::try_from(mode).ok())
        }
    }
}

fn local_path(data: &WireMap, key: &'static str) -> Result<PathBuf, String> {
    let path = PathBuf::from(input::path(data, key)?);
    if !path.is_absolute() {
        return Err(format!("ssh: {key} must be absolute"));
    }
    Ok(path)
}

/// Expands a leading `~` to the login directory (what `realpath(".")` is).
fn home_relative(path: &str) -> String {
    if path == "~" {
        ".".into()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("./{rest}")
    } else {
        path.to_owned()
    }
}

type Answer = Result<WireMap, String>;

pub fn call(
    shared: &Arc<Shared>,
    connection: &Arc<Connection>,
    method: &str,
    data: &WireMap,
    reply: Reply,
) {
    match method {
        "sftpUpload" | "sftpDownload" => {
            transfer(shared, connection, method == "sftpUpload", data, reply);
        }
        "sftpList" | "sftpStat" | "sftpMkdir" | "sftpRename" | "sftpRemove" | "sftpRead"
        | "sftpWrite" | "sftpRealpath" => {
            let request = match Request::parse(method, data) {
                Ok(request) => request,
                Err(error) => {
                    reply.failure(error);
                    return;
                }
            };
            let connection = Arc::clone(connection);
            let runtime = connection.runtime.clone();
            runtime.spawn(async move {
                let result = tokio::select! {
                    result = run(&connection, request) => result,
                    () = connection.stopping() => Err("ssh: session closed".into()),
                };
                match result {
                    Ok(map) => reply.map(&map),
                    Err(error) => reply.failure(error),
                }
            });
        }
        _ => reply.failure("ssh: unknown method"),
    }
}

enum Request {
    List(String),
    Stat(String, bool),
    Mkdir(String, Option<u32>),
    Rename(String, String),
    Remove(String, bool),
    Read(String, u32),
    Write(String, Vec<u8>, Option<u32>),
    Realpath(String),
}

impl Request {
    fn parse(method: &str, data: &WireMap) -> Result<Self, String> {
        let path = || input::path(data, "path").map(home_relative);
        Ok(match method {
            "sftpList" => Self::List(path()?),
            "sftpStat" => Self::Stat(path()?, !input::bool(data, "noFollow")),
            "sftpMkdir" => Self::Mkdir(path()?, mode(data)?),
            "sftpRename" => Self::Rename(
                home_relative(input::path(data, "from")?),
                home_relative(input::path(data, "to")?),
            ),
            "sftpRemove" => Self::Remove(path()?, input::bool(data, "recursive")),
            "sftpRead" => {
                let max = input::integer(
                    data,
                    "maxBytes",
                    MAX_BYTES_VALUE as i64,
                    1,
                    MAX_BYTES_VALUE as i64,
                )?;
                Self::Read(path()?, u32::try_from(max).unwrap_or(CHUNK))
            }
            "sftpWrite" => Self::Write(
                path()?,
                input::bytes(data, "bytes", MAX_BYTES_VALUE)?.to_vec(),
                mode(data)?,
            ),
            _ => Self::Realpath(path()?),
        })
    }
}

async fn run(connection: &Connection, request: Request) -> Answer {
    let sftp = connection.sftp().await?;
    let result = execute(&sftp, request).await;
    if let Err(SftpError::IO(_)) = &result {
        connection.sftp_broken(&sftp).await;
    }
    result.map_err(|error| sftp_error(&error))
}

async fn execute(sftp: &RawSftpSession, request: Request) -> Result<WireMap, SftpError> {
    let mut map = WireMap::new();
    match request {
        Request::List(path) => {
            let (entries, truncated) = list(sftp, &path).await?;
            map.insert("count", entries.len());
            map.insert("entries", serde_json::Value::Array(entries).to_string());
            map.insert("truncated", truncated);
        }
        Request::Stat(path, follow) => {
            let attributes = if follow {
                sftp.stat(path.as_str()).await?
            } else {
                sftp.lstat(path.as_str()).await?
            };
            let name = path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or("/");
            map.insert("name", name);
            insert_attributes(&mut map, &attributes.attrs);
        }
        Request::Mkdir(path, mode) => {
            let attributes = FileAttributes {
                permissions: mode,
                ..FileAttributes::empty()
            };
            sftp.mkdir(path, attributes).await?;
        }
        Request::Rename(from, to) => {
            sftp.rename(from, to).await?;
        }
        Request::Remove(path, recursive) => remove(sftp, &path, recursive).await?,
        Request::Read(path, max) => {
            let (bytes, size, truncated) = read(sftp, &path, max).await?;
            map.insert("bytes", WireValue::Bytes(bytes));
            map.insert("size", i64::try_from(size).unwrap_or(i64::MAX));
            map.insert("truncated", truncated);
        }
        Request::Write(path, bytes, mode) => {
            let handle = open_write(sftp, &path, mode).await?;
            let mut offset = 0_u64;
            let result = async {
                for chunk in bytes.chunks(CHUNK as usize) {
                    sftp.write(handle.as_str(), offset, chunk.to_vec()).await?;
                    offset += chunk.len() as u64;
                }
                Ok::<_, SftpError>(())
            }
            .await;
            let closed = sftp.close(handle).await;
            result?;
            closed?;
            map.insert("bytes", bytes.len());
        }
        Request::Realpath(path) => {
            let name = sftp.realpath(path).await?;
            let path = name.files.into_iter().next().map(|file| file.filename);
            let path = path.ok_or_else(|| SftpError::UnexpectedBehavior("no path".into()))?;
            if path.len() > MAX_PATH {
                return Err(SftpError::Limited("path".into()));
            }
            map.insert("path", path);
        }
    }
    Ok(map)
}

async fn open_write(
    sftp: &RawSftpSession,
    path: &str,
    mode: Option<u32>,
) -> Result<String, SftpError> {
    let attributes = FileAttributes {
        permissions: mode,
        ..FileAttributes::empty()
    };
    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
    Ok(sftp.open(path, flags, attributes).await?.handle)
}

async fn list(
    sftp: &RawSftpSession,
    path: &str,
) -> Result<(Vec<serde_json::Value>, bool), SftpError> {
    let handle = sftp.opendir(path).await?.handle;
    let mut entries = Vec::new();
    let mut bytes = 2_usize;
    let mut truncated = false;
    let result = async {
        'read: loop {
            let name = match sftp.readdir(handle.as_str()).await {
                Ok(name) => name,
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(error) => return Err(error),
            };
            for file in name.files {
                if file.filename == "." || file.filename == ".." {
                    continue;
                }
                let entry = entry_json(&file.filename, &file.longname, &file.attrs);
                let size = entry.to_string().len() + 1;
                if entries.len() >= MAX_ENTRIES || bytes + size > MAX_LISTING_BYTES {
                    truncated = true;
                    break 'read;
                }
                bytes += size;
                entries.push(entry);
            }
        }
        Ok(())
    }
    .await;
    let _ = sftp.close(handle).await;
    result?;
    Ok((entries, truncated))
}

async fn read(
    sftp: &RawSftpSession,
    path: &str,
    max: u32,
) -> Result<(Vec<u8>, u64, bool), SftpError> {
    let handle = sftp
        .open(path, OpenFlags::READ, FileAttributes::empty())
        .await?
        .handle;
    let result = async {
        let size = sftp.fstat(handle.as_str()).await?.attrs.size.unwrap_or(0);
        let mut bytes = Vec::with_capacity(usize::try_from(size.min(u64::from(max))).unwrap_or(0));
        let max = max as usize;
        // One byte past the limit tells a truncated file from one that fits.
        while bytes.len() <= max {
            let wanted =
                u32::try_from((max + 1 - bytes.len()).min(CHUNK as usize)).unwrap_or(CHUNK);
            match sftp.read(handle.as_str(), bytes.len() as u64, wanted).await {
                Ok(data) if data.data.is_empty() => break,
                Ok(data) => bytes.extend_from_slice(&data.data),
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(error) => return Err(error),
            }
        }
        let truncated = bytes.len() > max;
        bytes.truncate(max);
        Ok((bytes, size, truncated))
    }
    .await;
    let _ = sftp.close(handle).await;
    result
}

async fn remove(sftp: &RawSftpSession, path: &str, recursive: bool) -> Result<(), SftpError> {
    let attributes = sftp.lstat(path).await?.attrs;
    if kind(&attributes) != "dir" {
        sftp.remove(path).await?;
        return Ok(());
    }
    if recursive {
        let mut visited = 0;
        remove_tree(sftp, path.trim_end_matches('/').to_owned(), 0, &mut visited).await?;
    }
    sftp.rmdir(path).await?;
    Ok(())
}

/// Empties a directory, depth first, without following links.
async fn remove_tree(
    sftp: &RawSftpSession,
    directory: String,
    depth: usize,
    visited: &mut usize,
) -> Result<(), SftpError> {
    if depth >= MAX_DEPTH {
        return Err(SftpError::Limited("depth".into()));
    }
    let handle = sftp.opendir(directory.as_str()).await?.handle;
    let mut children = Vec::new();
    let listed = async {
        loop {
            match sftp.readdir(handle.as_str()).await {
                Ok(name) => {
                    for file in name.files {
                        if file.filename == "." || file.filename == ".." {
                            continue;
                        }
                        *visited += 1;
                        if *visited > MAX_REMOVE {
                            return Err(SftpError::Limited("entries".into()));
                        }
                        children.push((file.filename, kind(&file.attrs) == "dir"));
                    }
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }
    .await;
    let _ = sftp.close(handle).await;
    listed?;
    for (name, is_dir) in children {
        let child = format!("{directory}/{name}");
        if is_dir {
            Box::pin(remove_tree(sftp, child.clone(), depth + 1, visited)).await?;
            sftp.rmdir(child).await?;
        } else {
            sftp.remove(child).await?;
        }
    }
    Ok(())
}

struct Registration {
    shared: Arc<Shared>,
    id: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        lock(&self.shared.transfers).remove(&self.id);
    }
}

fn transfer(
    shared: &Arc<Shared>,
    connection: &Arc<Connection>,
    upload: bool,
    data: &WireMap,
    reply: Reply,
) {
    let parsed = (|| {
        let local = local_path(data, "localPath")?;
        let remote = home_relative(input::path(data, "remotePath")?);
        Ok::<_, String>((local, remote, mode(data)?))
    })();
    let (local, remote, mode) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let Some(permit) = Permit::acquire(&connection.transfers, MAX_TRANSFERS) else {
        reply.failure("ssh: too many transfers running");
        return;
    };
    let id = uuid();
    let (cancel, cancelled) = watch::channel(false);
    lock(&shared.transfers).insert(
        id.clone(),
        Transfer {
            session: connection.id.clone(),
            cancel,
        },
    );
    let registration = Registration {
        shared: Arc::clone(shared),
        id: id.clone(),
    };
    let mut result = WireMap::new();
    result.insert("transfer", id.as_str());
    reply.map(&result);
    let connection = Arc::clone(connection);
    let runtime = connection.runtime.clone();
    runtime.spawn(async move {
        let _permit = permit;
        let _registration = registration;
        let mut progress = Progress::new(&connection, &id);
        let work = async {
            let sftp = connection.sftp().await?;
            let result = if upload {
                send(&sftp, &local, &remote, mode, &mut progress).await
            } else {
                receive(&sftp, &remote, &local, &mut progress).await
            };
            if matches!(&result, Err(Failure::Sftp(SftpError::IO(_)))) {
                connection.sftp_broken(&sftp).await;
            }
            result.map_err(|failure| failure.message())
        };
        let mut cancelled = cancelled;
        let outcome = tokio::select! {
            result = work => result,
            _ = cancelled.wait_for(|cancel| *cancel) => Err("cancelled".to_owned()),
            () = connection.stopping() => Err("ssh: session closed".to_owned()),
        };
        let event = match outcome {
            Ok(bytes) => Event::TransferDone {
                transfer: id.clone(),
                bytes,
            },
            Err(message) => Event::TransferFailed {
                transfer: id.clone(),
                message,
            },
        };
        connection.events.offer(event);
    });
}

enum Failure {
    Sftp(SftpError),
    Local(&'static str),
    Other(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Self::Sftp(error) => sftp_error(&error),
            Self::Local(message) => message.to_owned(),
            Self::Other(message) => message,
        }
    }
}

impl From<SftpError> for Failure {
    fn from(error: SftpError) -> Self {
        Self::Sftp(error)
    }
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

fn local_error(error: &std::io::Error) -> Failure {
    Failure::Local(match error.kind() {
        std::io::ErrorKind::NotFound => "local: not found",
        std::io::ErrorKind::PermissionDenied => "local: permission denied",
        std::io::ErrorKind::StorageFull => "local: disk full",
        _ => "local: io error",
    })
}

struct Progress<'a> {
    connection: &'a Connection,
    id: &'a str,
    last: Option<Instant>,
}

impl<'a> Progress<'a> {
    fn new(connection: &'a Connection, id: &'a str) -> Self {
        Self {
            connection,
            id,
            last: None,
        }
    }

    fn report(&mut self, bytes: u64, total: u64, force: bool) {
        let now = Instant::now();
        if !force && self.last.is_some_and(|last| now - last < PROGRESS_INTERVAL) {
            return;
        }
        self.last = Some(now);
        self.connection.events.offer(Event::TransferProgress {
            transfer: self.id.to_owned(),
            bytes,
            total,
        });
    }
}

/// Upload: reads the local file in order and keeps up to [`PIPELINE`] writes
/// in flight.
async fn send(
    sftp: &Arc<RawSftpSession>,
    local: &PathBuf,
    remote: &str,
    mode: Option<u32>,
    progress: &mut Progress<'_>,
) -> Result<u64, Failure> {
    let mut file = tokio::fs::File::open(local)
        .await
        .map_err(|error| local_error(&error))?;
    let total = file
        .metadata()
        .await
        .map_err(|error| local_error(&error))?
        .len();
    let handle: Arc<str> = open_write(sftp, remote, mode).await?.into();
    progress.report(0, total, true);
    let result = async {
        let mut pending: VecDeque<JoinHandle<Result<usize, SftpError>>> = VecDeque::new();
        let mut offset = 0_u64;
        let mut written = 0_u64;
        loop {
            let mut chunk = vec![0; CHUNK as usize];
            let mut filled = 0;
            while filled < chunk.len() {
                let read = file
                    .read(&mut chunk[filled..])
                    .await
                    .map_err(|error| local_error(&error))?;
                if read == 0 {
                    break;
                }
                filled += read;
            }
            if filled == 0 {
                break;
            }
            chunk.truncate(filled);
            if pending.len() >= PIPELINE
                && let Some(task) = pending.pop_front()
            {
                written += finished(task).await? as u64;
                progress.report(written, total, false);
            }
            let sftp = Arc::clone(sftp);
            let handle = Arc::clone(&handle);
            let at = offset;
            pending.push_back(tokio::spawn(async move {
                let length = chunk.len();
                sftp.write(&*handle, at, chunk).await.map(|_| length)
            }));
            offset += filled as u64;
        }
        while let Some(task) = pending.pop_front() {
            written += finished(task).await? as u64;
            progress.report(written, total, false);
        }
        Ok::<_, Failure>(written)
    }
    .await;
    let closed = sftp.close(&*handle).await;
    let written = result?;
    closed?;
    progress.report(written, total.max(written), true);
    Ok(written)
}

async fn finished<T>(task: JoinHandle<Result<T, SftpError>>) -> Result<T, Failure> {
    match task.await {
        Ok(result) => Ok(result?),
        Err(_) => Err(Failure::Local("transfer task failed")),
    }
}

/// Removes a partial download unless it was completed.
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Download: keeps up to [`PIPELINE`] reads in flight, writes them in order to
/// a `.part` file beside the target and renames it at the end.
async fn receive(
    sftp: &Arc<RawSftpSession>,
    remote: &str,
    local: &PathBuf,
    progress: &mut Progress<'_>,
) -> Result<u64, Failure> {
    let mut name = local.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let partial_path = local.with_file_name(name);
    let handle: Arc<str> = sftp
        .open(remote, OpenFlags::READ, FileAttributes::empty())
        .await?
        .handle
        .into();
    let result = async {
        let total = sftp.fstat(&*handle).await?.attrs.size.unwrap_or(0);
        let mut file = tokio::fs::File::create(&partial_path)
            .await
            .map_err(|error| local_error(&error))?;
        let partial = Partial(Some(partial_path.clone()));
        progress.report(0, total, true);
        let mut pending: VecDeque<JoinHandle<Result<Vec<u8>, SftpError>>> = VecDeque::new();
        let mut requested = 0_u64;
        let mut received = 0_u64;
        loop {
            while pending.len() < PIPELINE && (requested < total || pending.is_empty()) {
                let sftp = Arc::clone(sftp);
                let handle = Arc::clone(&handle);
                let at = requested;
                pending.push_back(tokio::spawn(async move {
                    match sftp.read(&*handle, at, CHUNK).await {
                        Ok(data) => Ok(data.data),
                        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                            Ok(Vec::new())
                        }
                        Err(error) => Err(error),
                    }
                }));
                requested += u64::from(CHUNK);
            }
            let Some(task) = pending.pop_front() else {
                break;
            };
            let data = finished(task).await?;
            if data.is_empty() {
                // Reads past the end that are still in flight are empty too.
                pending.clear();
                break;
            }
            if data.len() < CHUNK as usize {
                // A short read: the server returns less near the end, or the
                // file changed. Continue sequentially from what arrived.
                for task in pending.drain(..) {
                    task.abort();
                }
                requested = received + data.len() as u64;
            }
            file.write_all(&data)
                .await
                .map_err(|error| local_error(&error))?;
            received += data.len() as u64;
            progress.report(received, total.max(received), false);
        }
        file.flush().await.map_err(|error| local_error(&error))?;
        file.sync_all().await.map_err(|error| local_error(&error))?;
        drop(file);
        tokio::fs::rename(&partial_path, local)
            .await
            .map_err(|error| local_error(&error))?;
        let mut partial = partial;
        partial.0 = None;
        Ok::<_, Failure>((received, total))
    }
    .await;
    let _ = sftp.close(&*handle).await;
    let (received, total) = result?;
    progress.report(received, total.max(received), true);
    Ok(received)
}

pub fn cancel(shared: &Shared, data: &WireMap, reply: Reply) {
    match input::handle(data, "transfer") {
        Ok(id) => match lock(&shared.transfers).get(id) {
            Some(transfer) => {
                transfer.cancel();
                reply.done();
            }
            None => reply.failure("ssh: unknown transfer"),
        },
        Err(error) => reply.failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_become_short_names() {
        assert_eq!(status_name(StatusCode::NoSuchFile), "no-such-file");
        assert_eq!(
            status_name(StatusCode::PermissionDenied),
            "permission-denied"
        );
        assert_eq!(status_name(StatusCode::Eof), "eof");
    }

    #[test]
    fn long_names_give_owner_and_group() {
        assert_eq!(
            owner_group("-rw-r--r--    1 david    staff        12 Jan  1 00:00 a.txt"),
            (Some("david"), Some("staff"))
        );
        assert_eq!(owner_group(""), (None, None));
    }

    #[test]
    fn entries_carry_kind_permissions_and_names() {
        let attributes = FileAttributes {
            size: Some(12),
            permissions: Some(0o100_644),
            mtime: Some(1_700_000_000),
            uid: Some(1000),
            ..FileAttributes::empty()
        };
        let entry = entry_json(
            "a.txt",
            "-rw-r--r-- 1 david staff 12 Jan 1 a.txt",
            &attributes,
        );
        assert_eq!(entry["kind"], "file");
        assert_eq!(entry["permissions"], "0644");
        assert_eq!(entry["owner"], "david");
        assert_eq!(entry["size"], 12);
    }

    #[test]
    fn home_paths_and_local_paths() {
        assert_eq!(home_relative("~"), ".");
        assert_eq!(home_relative("~/logs"), "./logs");
        assert_eq!(home_relative("/var/~x"), "/var/~x");
        let mut map = WireMap::new();
        map.insert("localPath", "relative/file");
        assert!(local_path(&map, "localPath").is_err());
    }

    #[test]
    fn requests_are_bounded() {
        let mut map = WireMap::new();
        map.insert("path", "/tmp/a");
        map.insert("maxBytes", (MAX_BYTES_VALUE + 1) as i64);
        assert!(Request::parse("sftpRead", &map).is_err());
        let mut map = WireMap::new();
        map.insert("path", "/tmp/a");
        map.insert("bytes", WireValue::Bytes(vec![0; MAX_BYTES_VALUE + 1]));
        assert!(Request::parse("sftpWrite", &map).is_err());
        let mut map = WireMap::new();
        map.insert("path", "x".repeat(MAX_PATH + 1));
        assert!(Request::parse("sftpList", &map).is_err());
        let mut map = WireMap::new();
        map.insert("path", "/tmp/a");
        map.insert("mode", 0o10_000_i64);
        assert!(Request::parse("sftpMkdir", &map).is_err());
    }
}

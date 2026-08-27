#[cfg(target_os = "linux")]
mod linux {
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    const MAX_ACTIVATION_BYTES: usize = 64 * 1024;
    const ACTIVATION_ACK: &[u8] = b"PAM-DESKTOP-ACTIVATION-1\n";

    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Activation {
        pub arguments: Vec<String>,
    }

    pub enum Instance {
        Primary(InstanceGuard),
        Forwarded,
    }

    pub struct InstanceGuard {
        listener: Option<UnixListener>,
        path: PathBuf,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl InstanceGuard {
        pub fn acquire(identifier: &str, arguments: &[String]) -> Result<Instance, String> {
            let path = socket_path(identifier)?;
            match UnixListener::bind(&path) {
                Ok(listener) => Ok(Instance::Primary(Self {
                    listener: Some(secure_listener(listener, &path)?),
                    path,
                    stop: Arc::new(AtomicBool::new(false)),
                    thread: None,
                })),
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    if forward(&path, arguments).is_ok() {
                        return Ok(Instance::Forwarded);
                    }
                    std::fs::remove_file(&path).map_err(|remove_error| {
                        format!(
                            "cannot recover stale single-instance socket {} after {error}: {remove_error}",
                            path.display()
                        )
                    })?;
                    let listener = UnixListener::bind(&path).map_err(|bind_error| {
                        format!(
                            "cannot bind single-instance socket {}: {bind_error}",
                            path.display()
                        )
                    })?;
                    Ok(Instance::Primary(Self {
                        listener: Some(secure_listener(listener, &path)?),
                        path,
                        stop: Arc::new(AtomicBool::new(false)),
                        thread: None,
                    }))
                }
                Err(error) => Err(format!(
                    "cannot bind single-instance socket {}: {error}",
                    path.display()
                )),
            }
        }

        pub fn listen<F>(&mut self, handler: F) -> Result<(), String>
        where
            F: Fn(Activation) + Send + 'static,
        {
            let listener = self
                .listener
                .take()
                .ok_or_else(|| "single-instance listener was already started".to_owned())?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("cannot configure single-instance listener: {error}"))?;
            let stop = self.stop.clone();
            self.thread = Some(
                std::thread::Builder::new()
                    .name("pam-desktop-instance".to_owned())
                    .spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            match listener.accept() {
                                Ok((mut stream, _)) => {
                                    let mut bytes = Vec::new();
                                    if Read::by_ref(&mut stream)
                                        .take(MAX_ACTIVATION_BYTES as u64 + 1)
                                        .read_to_end(&mut bytes)
                                        .is_ok()
                                        && bytes.len() <= MAX_ACTIVATION_BYTES
                                        && let Ok(activation) = serde_json::from_slice(&bytes)
                                    {
                                        handler(activation);
                                        let _ = stream.write_all(ACTIVATION_ACK);
                                    }
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(Duration::from_millis(25));
                                }
                                Err(_) => break,
                            }
                        }
                    })
                    .map_err(|error| format!("cannot start single-instance listener: {error}"))?,
            );
            Ok(())
        }
    }

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn forward(path: &PathBuf, arguments: &[String]) -> Result<(), String> {
        let payload = serde_json::to_vec(&Activation {
            arguments: arguments.to_vec(),
        })
        .map_err(|error| format!("cannot encode application activation: {error}"))?;
        if payload.len() > MAX_ACTIVATION_BYTES {
            return Err("application activation exceeds 64 KiB".to_owned());
        }
        let mut stream = UnixStream::connect(path)
            .map_err(|error| format!("cannot contact the primary application instance: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| format!("cannot secure the activation response: {error}"))?;
        stream
            .write_all(&payload)
            .map_err(|error| format!("cannot forward application activation: {error}"))?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|error| format!("cannot finish application activation: {error}"))?;
        let mut ack = Vec::with_capacity(ACTIVATION_ACK.len());
        Read::by_ref(&mut stream)
            .take(ACTIVATION_ACK.len() as u64 + 1)
            .read_to_end(&mut ack)
            .map_err(|error| format!("cannot confirm application activation: {error}"))?;
        if ack != ACTIVATION_ACK {
            return Err(
                "primary application returned an invalid activation acknowledgement".to_owned(),
            );
        }
        Ok(())
    }

    fn socket_path(identifier: &str) -> Result<PathBuf, String> {
        let runtime = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(runtime) => PathBuf::from(runtime),
            None => private_runtime_directory()?,
        };
        if !runtime.is_dir() {
            return Err(format!(
                "runtime directory {} does not exist",
                runtime.display()
            ));
        }
        let digest = format!("{:x}", Sha256::digest(identifier.as_bytes()));
        Ok(runtime.join(format!("pam-desktop-{}.sock", &digest[..24])))
    }

    fn private_runtime_directory() -> Result<PathBuf, String> {
        let status = std::fs::read_to_string("/proc/self/status")
            .map_err(|error| format!("cannot resolve the current Linux user: {error}"))?;
        let uid = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("Uid:")
                    .and_then(|ids| ids.split_whitespace().next())
            })
            .ok_or_else(|| "cannot resolve the current Linux user identifier".to_owned())?;
        let path = std::env::temp_dir().join(format!("pam-desktop-{uid}"));
        match std::fs::create_dir(&path) {
            Ok(()) => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| {
                    format!(
                        "cannot protect runtime directory {}: {error}",
                        path.display()
                    )
                })?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "cannot create runtime directory {}: {error}",
                    path.display()
                ));
            }
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "cannot inspect runtime directory {}: {error}",
                path.display()
            )
        })?;
        let expected_uid = uid
            .parse::<u32>()
            .map_err(|error| format!("invalid Linux user identifier: {error}"))?;
        if !metadata.is_dir()
            || metadata.uid() != expected_uid
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(format!(
                "runtime directory {} is not private to the current user",
                path.display()
            ));
        }
        Ok(path)
    }

    fn secure_listener(listener: UnixListener, path: &PathBuf) -> Result<UnixListener, String> {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| {
                format!(
                    "cannot protect single-instance socket {}: {error}",
                    path.display()
                )
            },
        )?;
        Ok(listener)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn forwards_arguments_and_protects_the_instance_socket() {
            let identifier = format!(
                "dev.pam.lifecycle-test-{}-{}",
                std::process::id(),
                std::thread::current().name().unwrap_or("worker")
            );
            let path = socket_path(&identifier).expect("socket path should resolve");
            let Instance::Primary(mut primary) =
                InstanceGuard::acquire(&identifier, &[]).expect("primary should acquire endpoint")
            else {
                panic!("first instance must be primary");
            };
            let mode = std::fs::metadata(&path)
                .expect("socket metadata should exist")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);

            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            primary
                .listen(move |activation| {
                    sender
                        .send(activation.arguments)
                        .expect("test receiver should remain connected");
                })
                .expect("primary listener should start");
            let arguments = vec!["pam://open/document".to_owned(), "file.pam".to_owned()];
            assert!(matches!(
                InstanceGuard::acquire(&identifier, &arguments)
                    .expect("secondary should forward activation"),
                Instance::Forwarded
            ));
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("primary should receive activation"),
                arguments
            );
        }

        #[test]
        fn rejects_oversized_activation_envelopes() {
            let path = PathBuf::from("/tmp/unused-pam-desktop-test.sock");
            let arguments = vec!["x".repeat(MAX_ACTIVATION_BYTES)];
            assert!(forward(&path, &arguments).is_err());
        }
    }
}

#[cfg(target_os = "linux")]
#[allow(unused_imports, reason = "used by the Servo-enabled binary")]
pub use linux::{Instance, InstanceGuard};

#[cfg(any(not(target_os = "linux"), test))]
mod portable {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use interprocess::local_socket::{
        GenericNamespaced, Listener, ListenerNonblockingMode, ListenerOptions, Stream, prelude::*,
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    const MAX_ACTIVATION_BYTES: usize = 64 * 1024;
    const ACTIVATION_ACK: &[u8] = b"PAM-DESKTOP-ACTIVATION-1\n";

    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Activation {
        pub arguments: Vec<String>,
    }

    pub enum Instance {
        Primary(InstanceGuard),
        Forwarded,
    }

    pub struct InstanceGuard {
        listener: Option<Listener>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl InstanceGuard {
        pub fn acquire(identifier: &str, arguments: &[String]) -> Result<Instance, String> {
            let endpoint = endpoint(identifier);
            let name = endpoint
                .as_str()
                .to_ns_name::<GenericNamespaced>()
                .map_err(|error| format!("cannot name the single-instance endpoint: {error}"))?;
            match ListenerOptions::new()
                .name(name)
                .nonblocking(ListenerNonblockingMode::Accept)
                .create_sync()
            {
                Ok(listener) => Ok(Instance::Primary(Self {
                    listener: Some(listener),
                    stop: Arc::new(AtomicBool::new(false)),
                    thread: None,
                })),
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    forward(&endpoint, arguments)?;
                    Ok(Instance::Forwarded)
                }
                Err(error) => Err(format!("cannot bind single-instance endpoint: {error}")),
            }
        }

        pub fn listen<F>(&mut self, handler: F) -> Result<(), String>
        where
            F: Fn(Activation) + Send + 'static,
        {
            let listener = self
                .listener
                .take()
                .ok_or_else(|| "single-instance listener was already started".to_owned())?;
            let stop = self.stop.clone();
            self.thread = Some(
                std::thread::Builder::new()
                    .name("pam-desktop-instance".to_owned())
                    .spawn(move || listen(&listener, &stop, handler))
                    .map_err(|error| format!("cannot start single-instance listener: {error}"))?,
            );
            Ok(())
        }
    }

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn listen<F>(listener: &Listener, stop: &AtomicBool, handler: F)
    where
        F: Fn(Activation),
    {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok(mut stream) => {
                    if let Ok(bytes) = read_frame(&mut stream)
                        && let Ok(activation) = serde_json::from_slice(&bytes)
                    {
                        handler(activation);
                        let _ = stream.write_all(ACTIVATION_ACK);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
    }

    fn forward(endpoint: &str, arguments: &[String]) -> Result<(), String> {
        let payload = serde_json::to_vec(&Activation {
            arguments: arguments.to_vec(),
        })
        .map_err(|error| format!("cannot encode application activation: {error}"))?;
        if payload.len() > MAX_ACTIVATION_BYTES {
            return Err("application activation exceeds 64 KiB".to_owned());
        }
        let name = endpoint
            .to_ns_name::<GenericNamespaced>()
            .map_err(|error| format!("cannot name the primary application endpoint: {error}"))?;
        let mut stream = Stream::connect(name)
            .map_err(|error| format!("cannot contact the primary application instance: {error}"))?;
        let length = u32::try_from(payload.len())
            .map_err(|_| "application activation exceeds protocol capacity".to_owned())?;
        stream
            .write_all(&length.to_be_bytes())
            .and_then(|()| stream.write_all(&payload))
            .map_err(|error| format!("cannot forward application activation: {error}"))?;
        let mut acknowledgement = [0_u8; ACTIVATION_ACK.len()];
        stream
            .read_exact(&mut acknowledgement)
            .map_err(|error| format!("cannot confirm application activation: {error}"))?;
        if acknowledgement != ACTIVATION_ACK {
            return Err(
                "primary application returned an invalid activation acknowledgement".to_owned(),
            );
        }
        Ok(())
    }

    fn read_frame(stream: &mut Stream) -> Result<Vec<u8>, String> {
        let mut length = [0_u8; 4];
        stream
            .read_exact(&mut length)
            .map_err(|error| format!("cannot read application activation length: {error}"))?;
        let length = u32::from_be_bytes(length) as usize;
        if length > MAX_ACTIVATION_BYTES {
            return Err("application activation exceeds 64 KiB".to_owned());
        }
        let mut payload = vec![0_u8; length];
        stream
            .read_exact(&mut payload)
            .map_err(|error| format!("cannot read application activation: {error}"))?;
        Ok(payload)
    }

    fn endpoint(identifier: &str) -> String {
        let digest = format!("{:x}", Sha256::digest(identifier.as_bytes()));
        format!("pam-desktop-{}", &digest[..24])
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn portable_endpoint_forwards_and_acknowledges_activation() {
            let identifier = format!("dev.pam.portable-lifecycle-test-{}", std::process::id());
            let Instance::Primary(mut primary) =
                InstanceGuard::acquire(&identifier, &[]).expect("primary should acquire endpoint")
            else {
                panic!("first portable instance must be primary");
            };
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            primary
                .listen(move |activation| {
                    sender
                        .send(activation.arguments)
                        .expect("test receiver should remain connected");
                })
                .expect("portable listener should start");
            let arguments = vec!["pam://portable/open".to_owned()];
            assert!(matches!(
                InstanceGuard::acquire(&identifier, &arguments)
                    .expect("portable secondary should receive an acknowledgement"),
                Instance::Forwarded
            ));
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("portable primary should receive activation"),
                arguments
            );
        }

        #[test]
        fn portable_endpoint_is_stable_and_does_not_leak_identifier() {
            let first = endpoint("dev.pam.private-product");
            assert_eq!(first, endpoint("dev.pam.private-product"));
            assert_eq!(first.len(), "pam-desktop-".len() + 24);
            assert!(!first.contains("private-product"));
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(unused_imports, reason = "used by the Servo-enabled binary")]
pub use portable::{Instance, InstanceGuard};

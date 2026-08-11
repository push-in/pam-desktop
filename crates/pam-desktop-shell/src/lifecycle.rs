#[cfg(target_os = "linux")]
mod linux {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    const MAX_ACTIVATION_BYTES: usize = 64 * 1024;

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
                    listener: Some(listener),
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
                        listener: Some(listener),
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
            .write_all(&payload)
            .map_err(|error| format!("cannot forward application activation: {error}"))
    }

    fn socket_path(identifier: &str) -> Result<PathBuf, String> {
        let runtime =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        if !runtime.is_dir() {
            return Err(format!(
                "runtime directory {} does not exist",
                runtime.display()
            ));
        }
        let digest = format!("{:x}", Sha256::digest(identifier.as_bytes()));
        Ok(runtime.join(format!("pam-desktop-{}.sock", &digest[..24])))
    }
}

#[cfg(target_os = "linux")]
#[allow(unused_imports, reason = "used by the Servo-enabled binary")]
pub use linux::{Instance, InstanceGuard};

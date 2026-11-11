//! Local port forwarding (`ssh -L`): a listener on this machine whose
//! connections are carried to a host and port reachable from the server.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pam_desktop_ui::modules::Reply;
use pam_desktop_ui::modules::support::uuid;
use pam_desktop_ui::wiremap::WireMap;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::events::Event;
use super::session::{Connection, channel_error};
use super::{MAX_FORWARDS, Permit, Shared, input, lock};

/// Connections carried at once by one forward.
pub const MAX_CONNECTIONS: usize = 64;

pub struct Entry {
    pub session: String,
    stop: watch::Sender<bool>,
}

impl Entry {
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

struct Registration {
    shared: Arc<Shared>,
    id: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        lock(&self.shared.forwards).remove(&self.id);
    }
}

/// The address to listen on: an IP literal or `localhost`; only loopback
/// unless `allowNonLoopback` is set.
pub fn bind_address(host: &str, port: u16, allow_non_loopback: bool) -> Result<SocketAddr, String> {
    let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .map_err(|_| "ssh: bindHost must be an IP address or localhost".to_owned())?
    };
    if !ip.is_loopback() && !allow_non_loopback {
        return Err(
            "ssh: bindHost must be a loopback address unless allowNonLoopback is set".into(),
        );
    }
    Ok(SocketAddr::new(ip, port))
}

pub fn open(shared: &Arc<Shared>, connection: &Arc<Connection>, data: &WireMap, reply: Reply) {
    let parsed = (|| {
        let bind_host = input::text(data, "bindHost", 64)?.unwrap_or("127.0.0.1");
        let bind_port = input::port(data, "bindPort", 0)?;
        let address = bind_address(bind_host, bind_port, input::bool(data, "allowNonLoopback"))?;
        let remote_host = input::host(data, "remoteHost")?;
        let remote_port = input::port(data, "remotePort", 1)?;
        Ok::<_, String>((address, remote_host, remote_port))
    })();
    let (address, remote_host, remote_port) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            reply.failure(error);
            return;
        }
    };
    let Some(permit) = Permit::acquire(&connection.forwards, MAX_FORWARDS) else {
        reply.failure("ssh: too many forwards open");
        return;
    };
    let shared = Arc::clone(shared);
    let connection = Arc::clone(connection);
    let runtime = connection.runtime.clone();
    runtime.spawn(async move {
        let _permit = permit;
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                reply.failure(match error.kind() {
                    std::io::ErrorKind::AddrInUse => "forward: address in use",
                    std::io::ErrorKind::PermissionDenied => "forward: permission denied",
                    _ => "forward: cannot listen",
                });
                return;
            }
        };
        let port = listener.local_addr().map_or(0, |address| address.port());
        let id = uuid();
        let (stop, stopped) = watch::channel(false);
        lock(&shared.forwards).insert(
            id.clone(),
            Entry {
                session: connection.id.clone(),
                stop,
            },
        );
        let _registration = Registration {
            shared: Arc::clone(&shared),
            id: id.clone(),
        };
        let mut result = WireMap::new();
        result.insert("forward", id.as_str());
        result.insert("port", i64::from(port));
        reply.map(&result);
        connection.events.offer(Event::ForwardOpened {
            forward: id.clone(),
            port,
        });
        let failure = serve(
            &connection,
            &id,
            listener,
            &remote_host,
            remote_port,
            stopped,
        )
        .await;
        if let Some(message) = failure {
            connection.events.offer(Event::ForwardFailed {
                forward: id,
                message: message.into(),
            });
        }
    });
}

/// Accepts until stopped; answers why it stopped on its own.
async fn serve(
    connection: &Arc<Connection>,
    id: &str,
    listener: TcpListener,
    remote_host: &str,
    remote_port: u16,
    mut stopped: watch::Receiver<bool>,
) -> Option<&'static str> {
    let active = Arc::new(AtomicUsize::new(0));
    let mut failures = 0_u64;
    let mut tasks: JoinSet<Result<(), String>> = JoinSet::new();
    let report = |failures: u64, active: &AtomicUsize| {
        connection.events.offer(Event::ForwardConnections {
            forward: id.to_owned(),
            connections: active.load(Ordering::Acquire),
            failures,
        });
    };
    loop {
        tokio::select! {
            _ = stopped.wait_for(|stop| *stop) => return None,
            () = connection.stopping() => return None,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else {
                    return Some("forward: listener failed");
                };
                let Some(permit) = Permit::acquire(&active, MAX_CONNECTIONS) else {
                    drop(stream);
                    failures += 1;
                    report(failures, &active);
                    continue;
                };
                report(failures, &active);
                let connection = Arc::clone(connection);
                let remote_host = remote_host.to_owned();
                tasks.spawn(async move {
                    let _permit = permit;
                    carry(&connection, stream, peer, &remote_host, remote_port).await
                });
            }
            Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                if !matches!(done, Ok(Ok(()))) {
                    failures += 1;
                }
                report(failures, &active);
            }
        }
    }
}

async fn carry(
    connection: &Connection,
    mut stream: TcpStream,
    peer: SocketAddr,
    remote_host: &str,
    remote_port: u16,
) -> Result<(), String> {
    let channel = connection
        .handle
        .channel_open_direct_tcpip(
            remote_host,
            u32::from(remote_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await
        .map_err(|error| channel_error(&error))?;
    let _ = stream.set_nodelay(true);
    let mut remote = channel.into_stream();
    // Errors after the tunnel opened are the peers closing; not a failure.
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
    Ok(())
}

pub fn stop(shared: &Shared, data: &WireMap, reply: Reply) {
    match input::handle(data, "forward") {
        Ok(id) => {
            if let Some(entry) = lock(&shared.forwards).get(id) {
                entry.stop();
            }
            reply.done();
        }
        Err(error) => reply.failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_only_to_loopback_unless_allowed() {
        assert!(bind_address("127.0.0.1", 0, false).is_ok());
        assert!(bind_address("localhost", 8080, false).is_ok());
        assert!(bind_address("::1", 0, false).is_ok());
        assert!(bind_address("[::1]", 0, false).is_ok());
        assert!(bind_address("0.0.0.0", 0, false).is_err());
        assert!(bind_address("192.168.1.2", 0, false).is_err());
        assert!(bind_address("0.0.0.0", 0, true).is_ok());
        assert!(bind_address("example.test", 0, true).is_err());
    }
}

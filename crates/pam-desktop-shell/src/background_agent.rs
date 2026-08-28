use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::{fmt::Write as _, io::Write as _};

use crate::event_hub::EventHub;
use crate::lifecycle::{Instance, InstanceGuard};
use crate::native::NativeServices;
use crate::project::Project;
use crate::runtime::DesktopRuntime;
use crate::scheduler::BackgroundScheduler;

const STOP_ARGUMENT: &str = "--pam-agent-stop";
const READY_ENVIRONMENT: &str = "PAM_DESKTOP_AGENT_READY_FILE";
const READY_MARKER: &[u8] = b"PAM-DESKTOP-AGENT-READY-1\n";
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub fn spawn(project: &Project, application_id: &str) -> Result<(), String> {
    if std::env::var_os("PAM_DESKTOP_AGENT_CHILD").is_some() {
        return Ok(());
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate desktop agent executable: {error}"))?;
    let readiness = readiness_path()?;
    let mut child = Command::new(executable)
        .arg("agent")
        .arg(project.root())
        .env("PAM_DESKTOP_AGENT_CHILD", "1")
        .env(READY_ENVIRONMENT, &readiness)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start background agent for {application_id}: {error}"))?;
    let deadline = std::time::Instant::now() + READY_TIMEOUT;
    loop {
        match std::fs::read(&readiness) {
            Ok(marker) if marker == READY_MARKER => {
                let _ = std::fs::remove_file(&readiness);
                return Ok(());
            }
            Ok(_) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&readiness);
                return Err("background agent returned an invalid readiness marker".to_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                let _ = child.kill();
                return Err(format!("cannot read background agent readiness: {error}"));
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot inspect background agent startup: {error}"))?
        {
            let _ = std::fs::remove_file(&readiness);
            return Err(format!(
                "background agent exited before readiness with {status}"
            ));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&readiness);
            return Err(format!(
                "background agent for {application_id} did not become ready within 15 seconds"
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

pub fn run(project: Project) -> Result<(), String> {
    let runtime = DesktopRuntime::prepare(project)?;
    let (project, supervisor, bootstrap) = runtime.into_worker_parts()?;
    if !bootstrap.workstation.background_agent {
        return Err("the desktop project has no background agent enabled".to_owned());
    }
    let agent_id = format!("{}.agent", bootstrap.manifest.identifier);
    let mut instance = match InstanceGuard::acquire(&agent_id, &[])? {
        Instance::Forwarded => {
            publish_readiness()?;
            return Ok(());
        }
        Instance::Primary(instance) => instance,
    };
    let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
    instance.listen(move |activation| {
        if activation
            .arguments
            .iter()
            .any(|value| value == STOP_ARGUMENT)
        {
            let _ = stop_sender.try_send(());
        }
    })?;

    let _native_services = NativeServices::prepare(
        project.root(),
        &bootstrap.manifest.identifier,
        &bootstrap.capabilities,
    )?;
    let supervisor = Arc::new(Mutex::new(supervisor));
    let events = EventHub::default();
    let _scheduler = BackgroundScheduler::start_headless(
        &bootstrap.background_jobs,
        &bootstrap.manifest.identifier,
        &supervisor,
        &events,
    )?;
    publish_readiness()?;
    stop_receiver
        .recv()
        .map_err(|_| "background agent control channel stopped".to_owned())?;
    drop(instance);
    Ok(())
}

fn readiness_path() -> Result<PathBuf, String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| format!("cannot generate background agent readiness token: {error}"))?;
    let mut token = String::with_capacity(random.len() * 2);
    for byte in random {
        write!(&mut token, "{byte:02x}")
            .map_err(|error| format!("cannot encode background agent readiness token: {error}"))?;
    }
    Ok(std::env::temp_dir().join(format!(
        "pam-desktop-agent-ready-{}-{token}",
        std::process::id(),
    )))
}

fn publish_readiness() -> Result<(), String> {
    let Some(path) = std::env::var_os(READY_ENVIRONMENT).map(PathBuf::from) else {
        return Ok(());
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("cannot publish background agent readiness: {error}"))?;
    file.write_all(READY_MARKER)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot persist background agent readiness: {error}"))
}

pub fn stop(application_id: &str) -> Result<(), String> {
    match InstanceGuard::acquire(
        &format!("{application_id}.agent"),
        &[STOP_ARGUMENT.to_owned()],
    )? {
        Instance::Forwarded => Ok(()),
        Instance::Primary(instance) => {
            drop(instance);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_stop_is_forwarded_to_the_private_agent_instance() {
        let application_id = format!("dev.pam.agent-test-{}", std::process::id());
        let mut guard = match InstanceGuard::acquire(&format!("{application_id}.agent"), &[])
            .expect("primary agent")
        {
            Instance::Primary(guard) => guard,
            Instance::Forwarded => panic!("test agent unexpectedly exists"),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        guard
            .listen(move |activation| {
                if activation.arguments == [STOP_ARGUMENT] {
                    sender.send(()).expect("stop signal");
                }
            })
            .expect("agent listener");
        stop(&application_id).expect("forward stop");
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("stop activation");
    }
}

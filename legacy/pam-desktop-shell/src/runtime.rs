use pam_desktop_protocol::Bootstrap;

use crate::project::Project;
#[cfg(feature = "gateway")]
use crate::startup_snapshot::StartupSnapshot;
use crate::worker::WorkerSupervisor;

pub struct DesktopRuntime {
    project: Project,
    supervisor: SupervisorPreparation,
    bootstrap: Bootstrap,
    snapshot_hit: bool,
    #[cfg(feature = "servo-engine")]
    startup_snapshot: Option<StartupSnapshot>,
}

enum SupervisorPreparation {
    Ready(Box<WorkerSupervisor>),
    #[cfg(feature = "servo-engine")]
    Starting(std::thread::JoinHandle<Result<WorkerSupervisor, String>>),
}

impl DesktopRuntime {
    pub fn prepare(project: Project) -> Result<Self, String> {
        #[cfg(feature = "gateway")]
        let snapshot = StartupSnapshot::prepare(&project)?;
        #[cfg(feature = "gateway")]
        let cached = snapshot.load(&project);
        let supervisor = WorkerSupervisor::start(project.clone())?;
        let bootstrap = supervisor.bootstrap().clone();
        project.validate_bootstrap(&bootstrap)?;
        #[cfg(feature = "gateway")]
        snapshot.publish(&bootstrap)?;
        #[cfg(feature = "gateway")]
        let snapshot_hit = cached.as_ref() == Some(&bootstrap);
        #[cfg(not(feature = "gateway"))]
        let snapshot_hit = false;

        Ok(Self {
            project,
            supervisor: SupervisorPreparation::Ready(Box::new(supervisor)),
            bootstrap,
            snapshot_hit,
            #[cfg(feature = "servo-engine")]
            startup_snapshot: None,
        })
    }

    #[cfg(feature = "servo-engine")]
    pub fn prepare_interactive(project: Project) -> Result<Self, String> {
        let snapshot = StartupSnapshot::prepare(&project)?;
        let Some(bootstrap) = snapshot.load(&project) else {
            return Self::prepare(project);
        };
        let worker_project = project.clone();
        let supervisor = std::thread::Builder::new()
            .name("pam-desktop-startup-worker".to_owned())
            .spawn(move || WorkerSupervisor::start(worker_project))
            .map_err(|error| format!("cannot start parallel PHP bootstrap: {error}"))?;
        Ok(Self {
            project,
            supervisor: SupervisorPreparation::Starting(supervisor),
            bootstrap,
            snapshot_hit: true,
            startup_snapshot: Some(snapshot),
        })
    }

    #[must_use]
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }

    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    #[must_use]
    pub fn entry(&self) -> std::path::PathBuf {
        self.project
            .resolve_entry(&self.bootstrap.windows[0].entry)
            .expect("the bootstrap entry was validated during preparation")
    }

    #[must_use]
    pub fn worker_generation(&self) -> u64 {
        match &self.supervisor {
            SupervisorPreparation::Ready(supervisor) => supervisor.generation(),
            #[cfg(feature = "servo-engine")]
            SupervisorPreparation::Starting(_) => 0,
        }
    }

    #[must_use]
    pub fn startup_snapshot_hit(&self) -> bool {
        self.snapshot_hit
    }

    #[cfg(feature = "gateway")]
    pub fn into_worker_parts(self) -> Result<(Project, WorkerSupervisor, Bootstrap), String> {
        let supervisor = match self.supervisor {
            SupervisorPreparation::Ready(supervisor) => *supervisor,
            #[cfg(feature = "servo-engine")]
            SupervisorPreparation::Starting(thread) => thread
                .join()
                .map_err(|_| "parallel PHP bootstrap thread panicked".to_owned())??,
        };
        let bootstrap = supervisor.bootstrap().clone();
        self.project.validate_bootstrap(&bootstrap)?;
        Ok((self.project, supervisor, bootstrap))
    }

    #[cfg(feature = "servo-engine")]
    pub fn into_parts(self) -> Result<(Project, WorkerSupervisor, Bootstrap, bool), String> {
        let Self {
            project,
            supervisor,
            bootstrap: cached_bootstrap,
            snapshot_hit,
            startup_snapshot,
        } = self;
        let supervisor = match supervisor {
            SupervisorPreparation::Ready(supervisor) => *supervisor,
            SupervisorPreparation::Starting(thread) => thread
                .join()
                .map_err(|_| "parallel PHP bootstrap thread panicked".to_owned())??,
        };
        let bootstrap = supervisor.bootstrap().clone();
        project.validate_bootstrap(&bootstrap)?;
        if let Some(snapshot) = &startup_snapshot {
            snapshot.publish(&bootstrap)?;
        }
        let snapshot_hit = snapshot_hit && cached_bootstrap == bootstrap;
        Ok((project, supervisor, bootstrap, snapshot_hit))
    }
}

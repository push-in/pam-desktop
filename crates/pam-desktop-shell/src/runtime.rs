use pam_desktop_protocol::Bootstrap;

use crate::project::Project;
use crate::worker::WorkerSupervisor;

pub struct DesktopRuntime {
    project: Project,
    supervisor: WorkerSupervisor,
    bootstrap: Bootstrap,
}

impl DesktopRuntime {
    pub fn prepare(project: Project) -> Result<Self, String> {
        let supervisor = WorkerSupervisor::start(project.clone())?;
        let bootstrap = supervisor.bootstrap().clone();
        project.validate_bootstrap(&bootstrap)?;

        Ok(Self {
            project,
            supervisor,
            bootstrap,
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
        self.supervisor.generation()
    }

    #[cfg(feature = "servo-engine")]
    pub fn into_parts(self) -> (Project, WorkerSupervisor, Bootstrap) {
        (self.project, self.supervisor, self.bootstrap)
    }
}

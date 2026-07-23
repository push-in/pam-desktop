use pam_desktop_protocol::Bootstrap;

use crate::project::Project;
use crate::worker::WorkerClient;

pub struct DesktopRuntime {
    project: Project,
    _worker: WorkerClient,
    bootstrap: Bootstrap,
}

impl DesktopRuntime {
    pub fn prepare(project: Project) -> Result<Self, String> {
        let mut worker = WorkerClient::spawn(&project)?;
        let bootstrap = worker.boot()?;
        bootstrap.window.validate()?;
        project.resolve_entry(&bootstrap.entry)?;

        Ok(Self {
            project,
            _worker: worker,
            bootstrap,
        })
    }

    #[must_use]
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }

    #[must_use]
    pub fn entry(&self) -> std::path::PathBuf {
        self.project.root().join(&self.bootstrap.entry)
    }

    #[cfg(feature = "servo-engine")]
    pub fn into_parts(self) -> (Project, WorkerClient, Bootstrap) {
        (self.project, self._worker, self.bootstrap)
    }
}

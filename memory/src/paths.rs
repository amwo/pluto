use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stores_dir(&self) -> PathBuf {
        self.root.join("memory_stores")
    }

    pub fn store_dir(&self, store: &str) -> PathBuf {
        self.stores_dir().join(store)
    }

    pub fn audit_log(&self) -> PathBuf {
        self.root.join("audit").join("memory_history.jsonl")
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.root.join("audit").join("objects")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn dreaming_dir(&self) -> PathBuf {
        self.root.join("dreaming")
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.dreaming_dir().join("jobs")
    }

    pub fn job_dir(&self, job_id: &str) -> PathBuf {
        self.jobs_dir().join(job_id)
    }
}

use crate::prelude::*;

pub struct Dirs {
    project_dirs: directories::ProjectDirs,
}

impl Dirs {
    pub fn new() -> Self {
        // TODO: fall back to /var, /etc, etc if we're running without $HOME
        // set
        Self {
            project_dirs: directories::ProjectDirs::from("", "", "teleterm")
                .expect("failed to find valid home directory"),
        }
    }

    pub fn create_all(&self) -> Result<()> {
        std::fs::create_dir_all(self.data_dir())
            .context(crate::error::CreateDir)?;
        Ok(())
    }

    fn config_dir(&self) -> &std::path::Path {
        self.project_dirs.config_dir()
    }

    pub fn config_file(&self, name: &str) -> std::path::PathBuf {
        self.config_dir().join(name)
    }

    fn data_dir(&self) -> &std::path::Path {
        self.project_dirs.data_dir()
    }

    pub fn data_file(&self, name: &str) -> std::path::PathBuf {
        self.data_dir().join(name)
    }
}

use crate::prelude::*;

pub struct Dirs {
    project_dirs: Option<directories::ProjectDirs>,
}

impl Dirs {
    pub fn new() -> Self {
        Self {
            project_dirs: directories::ProjectDirs::from("", "", "teleterm"),
        }
    }

    pub fn create_all(&self) -> Result<()> {
        if let Some(filename) = self.data_dir() {
            std::fs::create_dir_all(filename).with_context(|| {
                crate::error::CreateDir {
                    filename: filename.to_string_lossy(),
                }
            })?;
        }
        Ok(())
    }

    fn has_home(&self) -> bool {
        directories::BaseDirs::new().map_or(false, |dirs| {
            dirs.home_dir() != std::path::Path::new("/")
        })
    }

    fn global_config_dir(&self) -> &std::path::Path {
        std::path::Path::new("/etc/teleterm")
    }

    fn config_dir(&self) -> Option<&std::path::Path> {
        if self.has_home() {
            self.project_dirs
                .as_ref()
                .map(directories::ProjectDirs::config_dir)
        } else {
            None
        }
    }

    pub fn config_file(
        &self,
        name: &str,
        must_exist: bool,
    ) -> Option<std::path::PathBuf> {
        if let Some(config_dir) = self.config_dir() {
            let file = config_dir.join(name);
            if !must_exist || file.exists() {
                return Some(file);
            }
        }

        let file = self.global_config_dir().join(name);
        if !must_exist || file.exists() {
            return Some(file);
        }

        None
    }

    fn global_data_dir(&self) -> &std::path::Path {
        std::path::Path::new("/var/lib/teleterm")
    }

    fn data_dir(&self) -> Option<&std::path::Path> {
        if self.has_home() {
            self.project_dirs
                .as_ref()
                .map(directories::ProjectDirs::data_dir)
        } else {
            None
        }
    }

    pub fn data_file(
        &self,
        name: &str,
        must_exist: bool,
    ) -> Option<std::path::PathBuf> {
        if let Some(data_dir) = self.data_dir() {
            let file = data_dir.join(name);
            if !must_exist || file.exists() {
                return Some(file);
            }
        }

        let file = self.global_data_dir().join(name);
        if !must_exist || file.exists() {
            return Some(file);
        }

        None
    }
}

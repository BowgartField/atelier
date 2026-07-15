use crate::{BackendError, BackendErrorCode};
use std::path::PathBuf;

pub trait AppPaths: Send + Sync {
    fn data_dir(&self) -> Result<PathBuf, BackendError>;
    fn config_dir(&self) -> Result<PathBuf, BackendError>;
    fn cache_dir(&self) -> Result<PathBuf, BackendError>;
    fn resource_dir(&self) -> Result<PathBuf, BackendError>;
}

#[derive(Clone, Debug)]
pub struct ResolvedAppPaths {
    data_dir: PathBuf,
    config_dir: PathBuf,
    cache_dir: PathBuf,
    resource_dir: PathBuf,
}

impl ResolvedAppPaths {
    pub fn new(
        data_dir: PathBuf,
        config_dir: PathBuf,
        cache_dir: PathBuf,
        resource_dir: PathBuf,
    ) -> Self {
        Self {
            data_dir,
            config_dir,
            cache_dir,
            resource_dir,
        }
    }
}

impl AppPaths for ResolvedAppPaths {
    fn data_dir(&self) -> Result<PathBuf, BackendError> {
        Ok(self.data_dir.clone())
    }

    fn config_dir(&self) -> Result<PathBuf, BackendError> {
        Ok(self.config_dir.clone())
    }

    fn cache_dir(&self) -> Result<PathBuf, BackendError> {
        Ok(self.cache_dir.clone())
    }

    fn resource_dir(&self) -> Result<PathBuf, BackendError> {
        Ok(self.resource_dir.clone())
    }
}

#[derive(Clone, Debug)]
pub struct HeadlessAppPaths {
    resolved: ResolvedAppPaths,
}

impl HeadlessAppPaths {
    pub fn resolve(app_identifier: &str) -> Result<Self, BackendError> {
        Self::resolve_with_data_dir(app_identifier, None)
    }

    pub fn resolve_with_data_dir(
        app_identifier: &str,
        data_dir_override: Option<PathBuf>,
    ) -> Result<Self, BackendError> {
        let missing = |kind: &str| {
            BackendError::new(
                BackendErrorCode::NotReady,
                format!("Unable to resolve the system {kind} directory"),
            )
        };
        let data_dir = match data_dir_override {
            Some(path) => path,
            None => dirs::data_dir()
                .ok_or_else(|| missing("data"))?
                .join(app_identifier),
        };
        let config_dir = dirs::config_dir()
            .ok_or_else(|| missing("config"))?
            .join(app_identifier);
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| missing("cache"))?
            .join(app_identifier);
        let resource_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(ToOwned::to_owned))
            .unwrap_or_else(|| PathBuf::from("."));

        Ok(Self {
            resolved: ResolvedAppPaths::new(data_dir, config_dir, cache_dir, resource_dir),
        })
    }

    pub fn ensure_directories(&self) -> Result<(), BackendError> {
        std::fs::create_dir_all(self.data_dir()?)?;
        std::fs::create_dir_all(self.config_dir()?)?;
        std::fs::create_dir_all(self.cache_dir()?)?;
        Ok(())
    }
}

impl AppPaths for HeadlessAppPaths {
    fn data_dir(&self) -> Result<PathBuf, BackendError> {
        self.resolved.data_dir()
    }

    fn config_dir(&self) -> Result<PathBuf, BackendError> {
        self.resolved.config_dir()
    }

    fn cache_dir(&self) -> Result<PathBuf, BackendError> {
        self.resolved.cache_dir()
    }

    fn resource_dir(&self) -> Result<PathBuf, BackendError> {
        self.resolved.resource_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_data_dir_preserves_desktop_layout_root() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = HeadlessAppPaths::resolve_with_data_dir(
            "com.jean.desktop",
            Some(temp.path().join("com.jean.desktop")),
        )
        .expect("paths");
        assert_eq!(
            paths.data_dir().expect("data dir"),
            temp.path().join("com.jean.desktop")
        );
    }

    #[test]
    fn resolved_paths_are_returned_without_hidden_runtime_state() {
        let paths = ResolvedAppPaths::new(
            PathBuf::from("data"),
            PathBuf::from("config"),
            PathBuf::from("cache"),
            PathBuf::from("resources"),
        );
        assert_eq!(paths.data_dir().unwrap(), PathBuf::from("data"));
        assert_eq!(paths.config_dir().unwrap(), PathBuf::from("config"));
        assert_eq!(paths.cache_dir().unwrap(), PathBuf::from("cache"));
        assert_eq!(paths.resource_dir().unwrap(), PathBuf::from("resources"));
    }
}

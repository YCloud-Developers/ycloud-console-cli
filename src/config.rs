use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub dashboard: DashboardConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthConfig {
    pub token_type: String,
    pub access_token: String,
    pub refresh_token: String,
    pub record_id: String,
    pub requested_permissions: Vec<String>,
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("failed to parse config at {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }
        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        let temporary = temporary_path(path);
        let result = (|| -> Result<()> {
            let mut file = create_private_file(&temporary)?;
            file.write_all(raw.as_bytes())
                .with_context(|| format!("failed to write config at {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync config at {}", temporary.display()))?;
            restrict_owner_only(&temporary)?;
            fs::rename(&temporary, path)
                .with_context(|| format!("failed to replace config at {}", path.display()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config");
    path.with_file_name(format!(".{file_name}.{:032x}.tmp", rand::random::<u128>()))
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create temporary config at {}", path.display()))
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create temporary config at {}", path.display()))
}

pub fn remove(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove config at {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to set config permissions for {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_as_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let config = Config {
            dashboard: DashboardConfig {
                base_url: "http://127.0.0.1:8036".to_string(),
            },
            auth: AuthConfig {
                token_type: "Bearer".to_string(),
                access_token: "YCLI.access".to_string(),
                refresh_token: "YCLI.refresh".to_string(),
                record_id: "record-1".to_string(),
                requested_permissions: vec!["yc.identity.current.read".to_string()],
                tenant_id: Some("tenant-1".to_string()),
                user_id: Some("user-1".to_string()),
            },
        };

        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }
}

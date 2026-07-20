/*
 * Copyright 2026 sukawasatoru
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::Opt;
use anyhow::{Context as _, Result as Fallible};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub interval_secs: u64,
    pub otel_logs_endpoint: Option<Url>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_secs: 300,
            otel_logs_endpoint: None,
        }
    }
}

impl Config {
    pub fn config_path() -> Fallible<PathBuf> {
        let dirs = directories::ProjectDirs::from("com", "sukawasatoru", "crystal-disk-info-otel")
            .context("failed to resolve project directories")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    pub fn load(path: &Path) -> Fallible<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn merge(&mut self, opt: &Opt) {
        if let Some(interval_secs) = opt.interval_secs {
            self.interval_secs = interval_secs;
        }
        if let Some(endpoint) = opt.otel_logs_endpoint.clone() {
            self.otel_logs_endpoint = Some(endpoint);
        }
    }

    pub fn save(&self, path: &Path) -> Fallible<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let tmp_path = path.with_extension("tmp");
        {
            use std::io::Write as _;

            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options
                .open(&tmp_path)
                .with_context(|| format!("failed to open {}", tmp_path.display()))?;
            file.write_all(toml::to_string(self)?.as_bytes())
                .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        }
        std::fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn default_values() {
        let config = Config::default();
        assert_eq!(config.interval_secs, 300);
        assert!(config.otel_logs_endpoint.is_none());
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let config = Config::load(&path).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "interval_secs = [").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn merge_overwrites_only_some_fields() {
        let mut config = Config {
            interval_secs: 300,
            otel_logs_endpoint: Some(Url::parse("http://localhost:4318/v1/logs").unwrap()),
        };
        let opt = Opt {
            cmd: None,
            interval_secs: Some(60),
            otel_logs_endpoint: None,
        };
        config.merge(&opt);
        assert_eq!(config.interval_secs, 60);
        assert_eq!(
            config.otel_logs_endpoint,
            Some(Url::parse("http://localhost:4318/v1/logs").unwrap())
        );

        let opt = Opt {
            cmd: None,
            interval_secs: None,
            otel_logs_endpoint: Some(Url::parse("http://example.com/v1/logs").unwrap()),
        };
        config.merge(&opt);
        assert_eq!(config.interval_secs, 60);
        assert_eq!(
            config.otel_logs_endpoint,
            Some(Url::parse("http://example.com/v1/logs").unwrap())
        );
    }

    #[test]
    fn save_is_atomic_and_loadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            interval_secs: 120,
            otel_logs_endpoint: Some(Url::parse("http://127.0.0.1:4318/v1/logs").unwrap()),
        };
        config.save(&path).unwrap();
        assert!(!path.with_extension("tmp").exists());
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded, config);
    }

    #[test]
    fn save_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        Config::default().save(&path).unwrap();
        assert!(path.is_file());
        assert!(!path.with_extension("tmp").exists());
    }
}

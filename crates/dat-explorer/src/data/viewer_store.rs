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

use crate::model::viewer::{ViewerConfig, ViewerState};
use directories::ProjectDirs;
use rust_myscript::prelude::*;
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct ViewerStore {
    config_path: PathBuf,
    state_path: PathBuf,
}

impl ViewerStore {
    pub fn discover() -> Fallible<Self> {
        let dirs = ProjectDirs::from("com", "sukawasatoru", "dat-explorer")
            .context("no valid home directory")?;
        Ok(Self {
            config_path: dirs.config_dir().join("viewer.toml"),
            state_path: dirs
                .state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .join("viewer-state.toml"),
        })
    }

    pub fn load_config(&self) -> Fallible<ViewerConfig> {
        read_toml(&self.config_path)
    }

    pub fn load_state(&self) -> Fallible<ViewerState> {
        let state: ViewerState = read_toml(&self.state_path)?;
        ensure!(
            state.version == 1,
            "unsupported viewer state version: {}",
            state.version
        );
        Ok(state)
    }

    pub fn save_config(&self, config: &ViewerConfig) -> Fallible<()> {
        write_toml(&self.config_path, config)
    }

    pub fn save_state(&self, state: &ViewerState) -> Fallible<()> {
        write_toml(&self.state_path, state)
    }
}

fn read_toml<T: DeserializeOwned + Default>(path: &Path) -> Fallible<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("invalid TOML: {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn write_toml(path: &Path, value: &impl Serialize) -> Fallible<()> {
    let text = toml::to_string_pretty(value)?;
    let parent = path.parent().context("missing storage directory")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .with_context(|| format!("cannot save {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::viewer::{BackendKind, DirectoryState, ReadingPosition, ThreadState};

    #[test]
    fn state_roundtrip_and_replacement_preserve_directory_identity() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = ViewerStore {
            config_path: dir.path().join("config/viewer.toml"),
            state_path: dir.path().join("state/viewer-state.toml"),
        };
        assert_eq!(store.load_config().unwrap().backend, BackendKind::Direct);
        assert!(store.load_state().unwrap().threads.is_empty());
        store
            .save_config(&ViewerConfig {
                backend: BackendKind::Mcp,
            })
            .unwrap();
        assert_eq!(store.load_config().unwrap().backend, BackendKind::Mcp);
        let mut state = ViewerState::default();
        for directory in ["/日本語/one", "/日本語/two"] {
            state.directories.push(DirectoryState {
                path: directory.into(),
                selected_file: "board_1_123.dat".into(),
            });
            state.threads.push(ThreadState {
                directory: directory.into(),
                file: "board_1_123.dat".into(),
                position: ReadingPosition {
                    post_number: 42,
                    text_offset: 123,
                },
            });
        }
        store.save_state(&state).unwrap();
        state.threads[0].position.post_number = 50;
        store.save_state(&state).unwrap();
        let loaded = store.load_state().unwrap();
        assert_eq!(loaded.threads[0].position.post_number, 50);
        assert_eq!(loaded.threads[1].position.post_number, 42);
        assert_eq!(loaded.directories[1].path, Path::new("/日本語/two"));
        assert_eq!(store.load_config().unwrap().backend, BackendKind::Mcp);

        std::fs::write(&store.state_path, "version = 2").unwrap();
        assert!(store.load_state().is_err());
        std::fs::write(&store.state_path, "invalid = [").unwrap();
        assert!(store.load_state().is_err());
        assert_eq!(
            std::fs::read_to_string(&store.state_path).unwrap(),
            "invalid = ["
        );
    }
}

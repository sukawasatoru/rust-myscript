/*
 * Copyright 2023 sukawasatoru
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

use crate::model::Settings;
use rust_myscript::prelude::*;
use std::io::prelude::*;
use std::path::PathBuf;

pub trait PreferencesRepository {
    fn load_settings(&self) -> Fallible<Settings>;
    fn save_settings(&self, settings: &Settings) -> Fallible<()>;
}

pub trait GetPreferencesRepository {
    type Repo: PreferencesRepository;

    fn get_prefs_repo(&self) -> &Self::Repo;
}

pub struct PreferencesRepositoryImpl {
    config_dir_path: PathBuf,
}

impl PreferencesRepositoryImpl {
    pub fn create_with_path(config_dir_path: PathBuf) -> Self {
        Self { config_dir_path }
    }

    fn create_file_path(&self) -> PathBuf {
        self.config_dir_path.join("settings.toml")
    }
}

impl PreferencesRepository for PreferencesRepositoryImpl {
    fn load_settings(&self) -> Fallible<Settings> {
        if !self.config_dir_path.exists() {
            std::fs::create_dir_all(&self.config_dir_path)?;
        }

        let file_path = self.create_file_path();

        let mut settings_string = String::new();
        if file_path.exists() {
            let mut buf = std::io::BufReader::new(std::fs::File::open(file_path)?);
            buf.read_to_string(&mut settings_string)?;
        }

        Ok(toml::from_str(&settings_string)?)
    }

    fn save_settings(&self, settings: &Settings) -> Fallible<()> {
        if !self.config_dir_path.exists() {
            std::fs::create_dir_all(&self.config_dir_path)?;
        }

        let mut buf = std::io::BufWriter::new(std::fs::File::create(self.create_file_path())?);
        buf.write_all(toml::to_string(settings)?.as_bytes())?;
        buf.flush()?;

        Ok(())
    }
}

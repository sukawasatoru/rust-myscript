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

use crate::model::{DiskEntry, GadgetSnapshot, parse_temperature};
use anyhow::{Context as _, Result as Fallible};
use tracing::warn;
use windows_registry::CURRENT_USER;

pub const REGISTRY_PATH: &str = r"software\Crystal Dew World\CrystalDiskInfo";

pub fn read_snapshot() -> Fallible<Option<GadgetSnapshot>> {
    let key = match CURRENT_USER.open(REGISTRY_PATH) {
        Ok(key) => key,
        Err(e) if is_registry_not_found(&e) => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to open registry path {REGISTRY_PATH}"));
        }
    };

    let version = key.get_u32("Version").context("failed to read Version")?;
    if version != 1 {
        warn!(
            version,
            "unexpected CrystalDiskInfo gadget registry Version"
        );
    }

    let last_update = key
        .get_u32("LastUpdate")
        .context("failed to read LastUpdate")?;
    let disk_count = key
        .get_u32("DiskCount")
        .context("failed to read DiskCount")?;

    let mut disks = Vec::with_capacity(disk_count as usize);
    for i in 0..disk_count {
        let value_name = format!("Disk{i}");
        let model_serial = match key.get_string(&value_name) {
            Ok(value) => value,
            Err(e) => {
                warn!(?e, disk_index = i, "missing DiskN value; skip");
                continue;
            }
        };
        match read_disk_entry(&key, &model_serial) {
            Ok(entry) => disks.push(entry),
            Err(e) => {
                warn!(?e, model_serial, "failed to read disk entry; skip");
            }
        }
    }

    Ok(Some(GadgetSnapshot { last_update, disks }))
}

fn read_disk_entry(root: &windows_registry::Key, model_serial: &str) -> Fallible<DiskEntry> {
    let key = root
        .open(model_serial)
        .with_context(|| format!("failed to open disk key {model_serial}"))?;

    let drive_letter = key
        .get_string("DriveLetter")
        .context("failed to read DriveLetter")?;
    let model = key.get_string("Model").context("failed to read Model")?;
    let disk_size = key
        .get_string("DiskSize")
        .context("failed to read DiskSize")?;
    let temperature_raw = key
        .get_string("Temperature")
        .context("failed to read Temperature")?;
    let temperature_class = key
        .get_string("TemperatureClass")
        .context("failed to read TemperatureClass")?;
    let disk_status = key
        .get_u32("DiskStatus")
        .context("failed to read DiskStatus")?;

    Ok(DiskEntry {
        model_serial: model_serial.to_owned(),
        drive_letter,
        model,
        disk_size,
        temperature_celsius: parse_temperature(&temperature_raw),
        temperature_class,
        disk_status,
    })
}

fn is_registry_not_found<E: RegistryErrorCode>(e: &E) -> bool {
    is_not_found_hresult(e.hresult_i32())
}

/// Local adapter so `read_snapshot` does not touch HRESULT internals directly.
/// Implemented for the concrete error returned by `windows-registry`.
trait RegistryErrorCode {
    fn hresult_i32(&self) -> i32;
}

impl RegistryErrorCode for windows_result::Error {
    fn hresult_i32(&self) -> i32 {
        self.code().0
    }
}

/// `true` when `code` is HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND).
fn is_not_found_hresult(code: i32) -> bool {
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PATH_NOT_FOUND: u32 = 3;
    code == hresult_from_win32(ERROR_FILE_NOT_FOUND)
        || code == hresult_from_win32(ERROR_PATH_NOT_FOUND)
}

const fn hresult_from_win32(error: u32) -> i32 {
    if error as i32 <= 0 {
        error as i32
    } else {
        ((error & 0x0000_FFFF) | (7 << 16) | 0x8000_0000) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_not_found_hresult_codes() {
        assert!(is_not_found_hresult(hresult_from_win32(2)));
        assert!(is_not_found_hresult(hresult_from_win32(3)));
        assert!(!is_not_found_hresult(hresult_from_win32(5))); // ACCESS_DENIED
        assert!(!is_not_found_hresult(0));
    }

    #[test]
    fn is_registry_not_found_matches_file_and_path_not_found() {
        let file_not_found =
            windows_result::Error::from_hresult(windows_result::HRESULT::from_win32(2));
        let path_not_found =
            windows_result::Error::from_hresult(windows_result::HRESULT::from_win32(3));
        let access_denied =
            windows_result::Error::from_hresult(windows_result::HRESULT::from_win32(5));
        assert!(is_registry_not_found(&file_not_found));
        assert!(is_registry_not_found(&path_not_found));
        assert!(!is_registry_not_found(&access_denied));
    }
}

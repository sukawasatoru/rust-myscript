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

//! Manage the Perplexity API key using platform-specific credential stores.
//! Linux (including WSL) uses unencrypted SQLite in a dedicated directory.

use keyring_core::Entry;
use keyring_core::api::CredentialStoreApi;
use rust_myscript::prelude::*;

const SERVICE: &str = "rust-myscript.mcp-pplx";
const USER: &str = "perplexity-api-key";

pub(crate) fn entry() -> Fallible<Entry> {
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new()?;
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new()?;
    #[cfg(target_os = "linux")]
    let store = {
        let dirs = directories::ProjectDirs::from("com", "sukawasatoru", "mcp-pplx")
            .context("Could not determine the project directories")?;
        let state = dirs
            .state_dir()
            .context("Could not determine the state directory")?;
        sqlite_store(state)?
    };
    store
        .build(SERVICE, USER, None)
        .context("Failed to initialize the credential entry")
}

#[cfg(target_os = "linux")]
fn sqlite_store(dir: &std::path::Path) -> Fallible<std::sync::Arc<db_keystore::DbKeyStore>> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    // Restrict access by other users, including auxiliary files such as the WAL.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    db_keystore::DbKeyStore::new(db_keystore::DbKeyStoreConfig {
        path: dir.join("credentials.db"),
        // Avoid depending on the WSL kernel's support for io_uring.
        vfs: Some("syscall".into()),
        ..Default::default()
    })
    .context("Failed to initialize the SQLite credential store")
}

pub(crate) fn validate(key: &str) -> Fallible<()> {
    ensure!(
        !key.is_empty() && key.bytes().all(|b| b.is_ascii_graphic()),
        "The API key must be a nonempty ASCII string without whitespace or control characters"
    );
    Ok(())
}

pub(crate) fn save(entry: &Entry, key: &str) -> Fallible<()> {
    validate(key)?;
    entry
        .set_password(key)
        .context("Failed to save the API key")
}

pub(crate) fn load(entry: &Entry) -> Fallible<String> {
    let key = match entry.get_password() {
        Ok(key) => key,
        Err(keyring_core::Error::NoEntry) => {
            bail!("No API key is stored. Run `mcp-pplx auth set` to register one")
        }
        Err(err) => return Err(err).context("Failed to load the API key"),
    };
    validate(&key)?;
    Ok(key)
}

pub(crate) fn delete(entry: &Entry) -> Fallible<()> {
    entry
        .delete_credential()
        .context("Failed to delete the API key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_entry() -> Entry {
        keyring_core::mock::Store::new()
            .unwrap()
            .build(SERVICE, USER, None)
            .unwrap()
    }

    #[test]
    fn credentials_lifecycle() {
        let entry = mock_entry();
        assert!(load(&entry).unwrap_err().to_string().contains("auth set"));
        save(&entry, "test-key").unwrap();
        assert_eq!(load(&entry).unwrap(), "test-key");
        save(&entry, "replacement").unwrap();
        assert_eq!(load(&entry).unwrap(), "replacement");
        assert!(save(&entry, " ").is_err());
        assert_eq!(load(&entry).unwrap(), "replacement");
        delete(&entry).unwrap();
        assert!(load(&entry).is_err());
    }

    #[test]
    fn load_preserves_store_errors() {
        let entry = mock_entry();
        entry
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap()
            .set_error(keyring_core::Error::NoStorageAccess(Box::new(
                std::io::Error::other("locked"),
            )));
        let err = load(&entry).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<keyring_core::Error>(),
            Some(keyring_core::Error::NoStorageAccess(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sqlite_persists_across_store_reopen() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("mcp-pplx");
        {
            let store = sqlite_store(&dir).unwrap();
            assert!(!store.is_encrypted());
            save(&store.build(SERVICE, USER, None).unwrap(), "test-key").unwrap();
        }
        let store = sqlite_store(&dir).unwrap();
        let entry = store.build(SERVICE, USER, None).unwrap();
        assert_eq!(load(&entry).unwrap(), "test-key");
        assert_eq!(
            std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        delete(&entry).unwrap();
        assert!(load(&entry).is_err());
    }
}

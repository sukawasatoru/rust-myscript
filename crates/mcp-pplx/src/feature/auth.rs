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

//! Resolve API keys and run the synchronous authentication commands.
//! Async callers must run credential access and terminal input on a blocking thread.

use crate::data::credentials;
use rust_myscript::prelude::*;
use std::io::BufRead;

/// Use a CLI/environment override, or load the stored key when none is provided.
/// Invalid overrides fail without opening the credential store.
pub fn resolve_api_key(override_key: Option<String>) -> Fallible<String> {
    resolve(override_key, || credentials::load(&credentials::entry()?))
}

/// Save a key from stdin or a hidden terminal prompt and report success to stderr.
pub fn set(stdin: bool) -> Fallible<()> {
    let entry = credentials::entry()?;
    let key = if stdin {
        read_key(std::io::stdin().lock())?
    } else {
        rpassword::prompt_password("Perplexity API key: ")?
    };
    credentials::save(&entry, &key)?;
    eprintln!("API key saved.");
    Ok(())
}

/// Delete the stored key and report success to stderr.
pub fn delete() -> Fallible<()> {
    credentials::delete(&credentials::entry()?)?;
    eprintln!("API key deleted.");
    Ok(())
}

/// Resolve an option/environment override without opening the credential store.
fn resolve(
    override_key: Option<String>,
    load_stored: impl FnOnce() -> Fallible<String>,
) -> Fallible<String> {
    if let Some(key) = override_key {
        credentials::validate(&key)?;
        Ok(key)
    } else {
        load_stored()
    }
}

fn read_key(mut reader: impl BufRead) -> Fallible<String> {
    let mut key = String::new();
    reader
        .read_line(&mut key)
        .context("Failed to read the API key from stdin")?;
    if key.ends_with('\n') {
        key.pop();
        if key.ends_with('\r') {
            key.pop();
        }
    }
    credentials::validate(&key)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_bypasses_storage_for_overrides() {
        assert_eq!(
            resolve(Some("override".into()), || panic!(
                "store must not be opened"
            ))
            .unwrap(),
            "override"
        );
        for key in ["", "invalid key"] {
            assert!(resolve(Some(key.into()), || panic!("store must not be opened")).is_err());
        }
    }

    #[test]
    fn resolve_loads_storage_only_without_override() {
        assert_eq!(resolve(None, || Ok("stored".into())).unwrap(), "stored");
        assert_eq!(
            resolve(None, || bail!("store unavailable"))
                .unwrap_err()
                .to_string(),
            "store unavailable"
        );
    }

    #[test]
    fn read_key_handles_line_endings_and_rejects_invalid_input() {
        for input in ["test-key", "test-key\n", "test-key\r\n"] {
            assert_eq!(read_key(input.as_bytes()).unwrap(), "test-key");
        }
        for input in ["", "\n", " \n", "key with spaces", "key\r", "キー"] {
            assert!(read_key(input.as_bytes()).is_err());
        }
    }
}

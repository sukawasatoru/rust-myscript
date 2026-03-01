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
use blake3::{Hash, Hasher};
use rust_myscript::prelude::Fallible;
use std::path::Path;

pub fn compute_hash(path: &Path) -> Fallible<Hash> {
    let mut hasher = Hasher::new();
    // maybe_mmap_file use File.read if `file_size < 16 * 1024`.
    hasher.update_mmap(path)?;
    Ok(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn hash_file() {
        let temp_dir = tempdir().unwrap();
        let filepath = temp_dir.path().join("test.txt");
        std::fs::write(&filepath, b"Hello\n").unwrap();

        let actual = compute_hash(&filepath).unwrap().to_hex();
        assert_eq!(
            "38d5445421bfd60d4d48ff2a7acb3ed412e43e68e66cdb2bb86f604ec6e6caa0",
            actual.as_str(),
        );
    }
}

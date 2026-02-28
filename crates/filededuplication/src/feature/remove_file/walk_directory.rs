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
use rust_myscript::prelude::*;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::Sender;
use walkdir::WalkDir;

pub fn walk_directory(dir_root: &Path, tx: Sender<PathBuf>) {
    let walker = WalkDir::new(dir_root);
    for entry in walker {
        let entry = match entry {
            Ok(data) => data,
            Err(e) => {
                warn!(?e);
                continue;
            }
        };

        if entry.file_type().is_dir() {
            continue;
        }

        if entry.file_type().is_symlink() {
            debug!(path = %entry.path().display(), "symlink");
            continue;
        }

        if tx.blocking_send(entry.path().to_path_buf()).is_err() {
            debug!("tx is closed");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::remove_file::test_helpers::Target::*;
    use crate::feature::remove_file::test_helpers::create_target;
    use tokio::sync::mpsc::channel;
    use tokio::task::spawn_blocking;

    #[tokio::test]
    async fn skip_dir() {
        let tmp_dir = create_target(&[File("foo/bar"), Directory("piyo/hoge")]);

        let (tx, mut rx) = channel(100);
        let target = tmp_dir.path().to_owned();
        spawn_blocking(move || walk_directory(&target, tx));

        let mut actual = vec![];
        while let Some(data) = rx.recv().await {
            actual.push(data.strip_prefix(tmp_dir.path()).unwrap().to_owned());
        }

        assert_eq!(vec![PathBuf::from("foo/bar")], actual);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skip_symlink_file() {
        let tmp_dir = create_target(&[File("foo/bar"), Symlink("foo/bar", "piyo/hoge")]);

        let (tx, mut rx) = channel(100);
        let target = tmp_dir.path().to_owned();
        spawn_blocking(move || walk_directory(&target, tx));

        let mut actual = vec![];
        while let Some(data) = rx.recv().await {
            actual.push(data.strip_prefix(tmp_dir.path()).unwrap().to_owned());
        }

        assert_eq!(vec![PathBuf::from("foo/bar")], actual);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skip_symlink_dir() {
        let tmp_dir = create_target(&[File("foo/bar"), Symlink("foo", "piyo/hoge")]);

        let (tx, mut rx) = channel(100);
        let target = tmp_dir.path().to_owned();
        spawn_blocking(move || walk_directory(&target, tx));

        let target = tmp_dir.path();
        let mut actual = vec![];
        while let Some(data) = rx.recv().await {
            actual.push(data.strip_prefix(target).unwrap().to_owned());
        }

        assert_eq!(vec![PathBuf::from("foo/bar")], actual);
    }
}

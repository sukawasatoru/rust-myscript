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
use crate::feature::remove_file::compute_hash::compute_hash;
use crate::feature::remove_file::walk_directory::walk_directory;
use rust_myscript::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::available_parallelism;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::channel;
use tokio::task::JoinSet;

mod compute_hash;
mod walk_directory;

pub async fn remove_file(
    dry_run: bool,
    jobs: Option<usize>,
    master: PathBuf,
    shrink: PathBuf,
) -> Fallible<()> {
    ensure!(master != shrink, "master and shrink are same");
    ensure!(master.is_dir(), "master is not directory");
    ensure!(shrink.is_dir(), "shrink is not directory");

    let semaphore = Arc::new(Semaphore::new(jobs.unwrap_or_else(|| {
        (available_parallelism().map(|data| data.get()).unwrap_or(1) / 2).max(1)
    })));
    let (tx, mut rx) = channel::<PathBuf>(64);

    let walk_target = master.to_owned();
    let walker = tokio::task::spawn_blocking(move || walk_directory(&walk_target, tx));

    let mut join_set = JoinSet::new();

    while let Some(filepath) = rx.recv().await {
        let permit = semaphore.clone().acquire_owned().await?;
        let shrink_filepath = shrink.join(filepath.strip_prefix(&master)?);

        if shrink_filepath.exists() {
            join_set.spawn(async move {
                let _permit = permit;

                let lhs_target = filepath.to_owned();
                let lhs = tokio::task::spawn_blocking(move || compute_hash(&lhs_target));
                let rhs_target = shrink_filepath.to_owned();
                let rhs = tokio::task::spawn_blocking(move || compute_hash(&rhs_target));

                let (lhs_hash, rhs_hash) = tokio::try_join!(lhs, rhs)?;
                let lhs_hash = lhs_hash
                    .with_context(|| format!("failed to compute hash: {}", filepath.display()))?;
                let rhs_hash = rhs_hash.with_context(|| {
                    format!("failed to compute hash: {}", shrink_filepath.display())
                })?;

                if lhs_hash == rhs_hash {
                    if !dry_run {
                        std::fs::remove_file(&shrink_filepath).with_context(|| {
                            format!("failed to remove file: {}", shrink_filepath.display())
                        })?;
                    }
                    info!(path = %shrink_filepath.display(), "remove");
                    Ok(ShrinkResult::Remove)
                } else {
                    debug!(path = %shrink_filepath.display(), reason = %"different", "skip");
                    Ok(ShrinkResult::Skip)
                }
            });
        } else {
            debug!(path = %filepath.display(), reason = %"not found", "skip");
            join_set.spawn(std::future::ready(Ok(ShrinkResult::Skip)));
        }
    }

    let mut errors = vec![];

    if let Err(e) = walker.await.context("walk directory failed") {
        warn!(?e, "walk directory failed");
        errors.push(e);
    }

    let mut count_all = 0u32;
    let mut count_remove = 0u32;
    let mut count_skip = 0u32;
    while let Some(result) = join_set.join_next().await {
        count_all += 1;

        let result = match result.context("consumer thread panicked") {
            Ok(data) => data,
            Err(e) => {
                warn!(?e, "consumer thread panicked");
                errors.push(e);
                continue;
            }
        };

        match result {
            Ok(ShrinkResult::Remove) => count_remove += 1,
            Ok(ShrinkResult::Skip) => count_skip += 1,
            Err(err) => {
                error!(?err, "failed to shrink file");
                errors.push(err);
            }
        }
    }

    info!(
        all = count_all,
        remove = count_remove,
        skip = count_skip,
        error = errors.len(),
        "complete",
    );

    if !errors.is_empty() {
        warn!("error:");
        for entry in errors {
            warn!(?entry);
        }
        bail!("all file proceeded but some errors occurred");
    }

    Ok(())
}

enum ShrinkResult {
    Remove,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::remove_file::test_helpers::Target::*;
    use crate::feature::remove_file::test_helpers::{create_target, init_tracing};
    use tempfile::tempdir;

    #[tokio::test]
    pub async fn fail_if_master_and_shrink_are_same() {
        let temp_dir = tempdir().unwrap();

        remove_file(
            false,
            None,
            temp_dir.path().to_owned(),
            temp_dir.path().to_owned(),
        )
        .await
        .unwrap_err();
    }

    #[tokio::test]
    async fn fail_if_master_is_file() {
        let temp_dir1 = create_target(&[File("file")]);
        let temp_dir2 = tempdir().unwrap();

        remove_file(
            false,
            None,
            temp_dir1.path().join("file"),
            temp_dir2.path().to_owned(),
        )
        .await
        .unwrap_err();

        assert!(temp_dir1.path().join("file").exists());
    }

    #[tokio::test]
    async fn fail_if_shrink_is_file() {
        let temp_dir1 = create_target(&[]);
        let temp_dir2 = create_target(&[File("file")]);

        remove_file(
            false,
            None,
            temp_dir1.path().to_owned(),
            temp_dir2.path().join("file"),
        )
        .await
        .unwrap_err();

        assert!(temp_dir2.path().join("file").exists());
    }

    #[tokio::test]
    async fn write_log_if_skip_not_found() {
        let temp_dir1 = create_target(&[File("file")]);
        let temp_dir2 = tempdir().unwrap();

        let (_guard, log_writer) = init_tracing();

        remove_file(
            false,
            None,
            temp_dir1.path().to_owned(),
            temp_dir2.path().to_owned(),
        )
        .await
        .unwrap();

        let actual_messages = log_writer.remove();
        let mut actual_messages = actual_messages.trim_end().split('\n');
        assert_eq!(
            format!(
                "skip path={} reason=not found",
                temp_dir1.path().join("file").display(),
            ),
            actual_messages.next().unwrap(),
        );
        assert_eq!(
            "complete all=1 remove=0 skip=1 error=0",
            actual_messages.next().unwrap(),
        );
        assert!(actual_messages.next().is_none());

        assert!(temp_dir1.path().join("file").exists());
    }

    #[tokio::test]
    async fn skip_if_different_hash() {
        let temp_dir1 = create_target(&[File("file")]);
        std::fs::write(temp_dir1.path().join("file"), "different").unwrap();

        let temp_dir2 = create_target(&[File("file")]);

        let (_guard, log_writer) = init_tracing();

        remove_file(
            false,
            None,
            temp_dir1.path().to_owned(),
            temp_dir2.path().to_owned(),
        )
        .await
        .unwrap();

        let actual_messages = log_writer.remove();
        let mut actual_messages = actual_messages.trim_end().split('\n');
        assert_eq!(
            format!(
                "skip path={} reason=different",
                temp_dir2.path().join("file").display(),
            ),
            actual_messages.next().unwrap(),
        );
        assert_eq!(
            "complete all=1 remove=0 skip=1 error=0",
            actual_messages.next().unwrap(),
        );
        assert!(actual_messages.next().is_none());

        assert!(temp_dir1.path().join("file").exists());
        assert!(temp_dir2.path().join("file").exists());
    }

    #[tokio::test]
    async fn skip_if_dry_run() {
        let temp_dir1 = create_target(&[File("file")]);
        let temp_dir2 = create_target(&[File("file")]);

        let (_guard, log_writer) = init_tracing();

        remove_file(
            true,
            None,
            temp_dir1.path().to_owned(),
            temp_dir2.path().to_owned(),
        )
        .await
        .unwrap();

        let actual_messages = log_writer.remove();
        let mut actual_messages = actual_messages.trim_end().split('\n');
        assert_eq!(
            format!("remove path={}", temp_dir2.path().join("file").display()),
            actual_messages.next().unwrap(),
        );
        assert_eq!(
            "complete all=1 remove=1 skip=0 error=0",
            actual_messages.next().unwrap(),
        );
        assert!(actual_messages.next().is_none());

        assert!(temp_dir1.path().join("file").exists());
        assert!(temp_dir2.path().join("file").exists());
    }

    #[tokio::test]
    async fn remove_file_successfully() {
        let temp_dir1 = create_target(&[File("file")]);
        let temp_dir2 = create_target(&[File("file")]);

        let (_guard, log_writer) = init_tracing();

        remove_file(
            false,
            None,
            temp_dir1.path().to_owned(),
            temp_dir2.path().to_owned(),
        )
        .await
        .unwrap();

        let actual_messages = log_writer.remove();
        let mut actual_messages = actual_messages.trim_end().split('\n');
        assert_eq!(
            format!("remove path={}", temp_dir2.path().join("file").display()),
            actual_messages.next().unwrap(),
        );
        assert_eq!(
            "complete all=1 remove=1 skip=0 error=0",
            actual_messages.next().unwrap(),
        );
        assert!(actual_messages.next().is_none());

        assert!(temp_dir1.path().join("file").exists());
        assert!(!temp_dir2.path().join("file").exists());
    }
}

#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers {
    use Target::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tempfile::{TempDir, tempdir};
    use tracing::dispatcher::DefaultGuard;
    use tracing_subscriber::fmt::MakeWriter;

    pub enum Target {
        File(&'static str),
        Directory(&'static str),
        Symlink(&'static str, &'static str),
    }

    pub fn create_target(targets: &[Target]) -> TempDir {
        let temp_dir = tempdir().unwrap();
        for target in targets {
            match target {
                File(pathname) => {
                    let abs_pathname = temp_dir.path().join(pathname);
                    std::fs::create_dir_all(abs_pathname.parent().unwrap()).unwrap();
                    std::fs::File::create(abs_pathname).unwrap();
                }
                Directory(pathname) => {
                    std::fs::create_dir_all(temp_dir.path().join(pathname)).unwrap();
                }
                Symlink(pathname, symlink_pathname) => {
                    let abs_pathname = temp_dir.path().join(pathname);
                    let abs_symlink_pathname = temp_dir.path().join(symlink_pathname);
                    std::fs::create_dir_all(abs_symlink_pathname.parent().unwrap()).unwrap();

                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(abs_pathname, abs_symlink_pathname).unwrap();
                    }
                }
            }
        }

        temp_dir
    }

    pub fn init_tracing() -> (DefaultGuard, LogWriter) {
        let writer = LogWriter::new();
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(writer.clone())
                .with_max_level(tracing::Level::TRACE)
                .with_level(false)
                .with_target(false)
                .without_time()
                .with_ansi(false)
                .finish(),
        );

        (guard, writer)
    }

    #[derive(Clone)]
    pub struct LogWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl LogWriter {
        fn new() -> Self {
            Self {
                buf: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn remove(&self) -> String {
            let mut buf = self.buf.lock().expect("poisoned");
            let string = std::str::from_utf8(&buf[..])
                .expect("invalid utf-8")
                .to_owned();
            buf.clear();
            string
        }
    }

    impl<'a> MakeWriter<'a> for LogWriter {
        type Writer = LogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buf.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

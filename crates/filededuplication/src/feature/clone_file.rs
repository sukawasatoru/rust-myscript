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
use blake2::{Blake2b512, Digest};
use futures::prelude::*;
use rust_myscript::prelude::*;
use std::collections::HashSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::io::AsyncReadExt;
use tracing::Instrument;

unsafe extern "C" {
    static errno: libc::c_int;

    fn clonefile(
        src: *const libc::c_schar,
        dst: *const libc::c_schar,
        flag: libc::c_int,
    ) -> libc::c_int;
}

pub async fn clone_file(
    dry_run: bool,
    jobs: Option<usize>,
    target_dir: Vec<PathBuf>,
    backup_dir: Option<PathBuf>,
    force: bool,
) -> Fallible<()> {
    if backup_dir.is_none() && !force {
        bail!("need to set backup directory or use force flag");
    }

    debug!(?target_dir);

    let mut files = HashSet::new();
    for target in &target_dir {
        files.extend(walk_dir(target).await?);
    }

    // launchctl limit maxfiles
    let job_num = jobs.unwrap_or_else(num_cpus::get).min(200);

    let mut files = files.into_iter().collect::<Vec<_>>();
    let window = files.len() / job_num;
    let mut futs = stream::FuturesUnordered::new();
    for index in 0..job_num {
        let entries = if index == job_num - 1 {
            std::mem::take(&mut files)
        } else {
            files.drain(0..window).collect::<Vec<_>>()
        };

        let fut = tokio::task::spawn(
            async move {
                let mut digest = Blake2b512::new();
                let mut buf = [0u8; 4096];
                let mut ret = Vec::with_capacity(entries.len());
                'entry: for entry in entries {
                    info!("calculate begin");
                    let source_file = tokio::fs::File::open(&entry).await.unwrap();
                    let mut reader = tokio::io::BufReader::new(source_file);

                    loop {
                        let n = match reader.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(e) => {
                                eprintln!("{e:?}");
                                warn!(?e);
                                continue 'entry;
                            }
                        };
                        digest.update(&buf[0..n]);
                    }

                    let hash = digest.finalize_reset().to_vec();
                    info!(hash = %HexFormat(&hash), "calculate end");
                    ret.push((entry, hash))
                }
                ret
            }
            .instrument(info_span!("fut", index)),
        );
        futs.push(fut);
    }

    let mut file_hash_map = std::collections::HashMap::<Vec<u8>, Vec<PathBuf>>::new();
    while let Some(data) = futs.next().await {
        for (entry, hash) in data? {
            match file_hash_map.get_mut(&hash) {
                Some(data) => data.push(entry),
                None => {
                    file_hash_map.insert(hash, vec![entry]);
                }
            }
        }
    }

    for value in file_hash_map.values() {
        if value.len() < 2 {
            continue;
        }

        info!(?value);
        let source = &value[0];
        for file_path in &value[1..value.len()] {
            if let Some(ref backup_dir_root) = backup_dir {
                let backup_path = if file_path.has_root() {
                    backup_dir_root.join(file_path.strip_prefix("/")?)
                } else {
                    backup_dir_root.join(file_path)
                };

                let backup_path_parent = backup_path.parent().context("parent")?;

                if dry_run {
                    eprintln!("Would create dir to {backup_path_parent:?}");
                    eprintln!("Would move to {backup_path_parent:?} from {file_path:?}");
                } else {
                    debug!(?backup_path_parent, "create");
                    let create_dir_ret = tokio::fs::create_dir_all(backup_path_parent).await;

                    if create_dir_ret.is_err() {
                        eprintln!("failed to create dir: {create_dir_ret:?}");
                        continue;
                    }

                    debug!(from = ?file_path, to = ?backup_path, "rename");
                    let move_ret = tokio::fs::rename(&file_path, &backup_path).await;
                    if move_ret.is_err() {
                        eprintln!("failed to move file: {move_ret:?}");
                        continue;
                    }
                }
            } else {
                // TODO: use repository.
                #[allow(clippy::collapsible_if)]
                if dry_run {
                    eprintln!("Would remove {file_path:?}");
                } else {
                    debug!(?file_path, "remove");
                    let ret_rm = tokio::fs::remove_file(file_path).await;
                    if ret_rm.is_err() {
                        eprintln!("failed to remove file: {ret_rm:?}");
                    }
                }
            }

            if dry_run {
                eprintln!("Would clone {source:?} to {file_path:?}");
            } else {
                println!("clone {source:?} to {file_path:?}");

                unsafe {
                    let ret_clonefile = clonefile(
                        CString::new(source.to_str().context("source")?)?.as_ptr(),
                        CString::new(file_path.to_str().context("file_path")?)?.as_ptr(),
                        0,
                    );
                    if ret_clonefile != 0 {
                        eprintln!("failed to execute the clonefile command: {errno}");
                    }
                };
            }
        }
    }

    info!("Bye");

    Ok(())
}

fn walk_dir(
    target_dir: &Path,
) -> Pin<Box<dyn '_ + Future<Output = anyhow::Result<HashSet<PathBuf>>>>> {
    Box::pin(async move {
        let mut read_dir = tokio::fs::read_dir(target_dir).await?;
        let mut files = HashSet::new();
        loop {
            let dir_entry = match read_dir.next_entry().await {
                Ok(None) => break,
                Ok(Some(data)) => data,
                Err(e) => {
                    eprintln!("failed to read entry: {e:?}");
                    continue;
                }
            };

            let dir_entry_path = dir_entry.path();
            let symlink_meta = tokio::fs::symlink_metadata(&dir_entry_path).await?;
            let symlink_file_type = symlink_meta.file_type();

            if symlink_file_type.is_symlink() {
                debug!(?dir_entry_path, "ignore symlink");
            } else if symlink_file_type.is_dir() {
                debug!(?dir_entry_path, "dir");
                let ret = walk_dir(&dir_entry_path).await?;
                files.extend(ret);
            } else if symlink_file_type.is_file() {
                debug!(?dir_entry_path, "file");
                if symlink_meta.permissions().readonly() {
                    debug!(?dir_entry_path, "readonly");
                    continue;
                }
                files.insert(dir_entry_path);
            } else {
                unreachable!();
            }
        }

        Ok(files)
    })
}

/*
 * Copyright 2023, 2025 sukawasatoru
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

use async_stream::try_stream;
use clap::{Parser, ValueEnum};
use futures::stream::FuturesOrdered;
use futures::{Stream, StreamExt, pin_mut};
use rust_myscript::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "infer")]
    method: MimeMethod,

    /// target file or directory.
    target: PathBuf,
}

#[derive(Clone, ValueEnum)]
enum MimeMethod {
    Infer,

    /// for debug.
    #[cfg(feature = "tree_magic_mini")]
    TreeMagic,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    let opt = Opt::parse();

    if !opt.target.exists() {
        bail!("target did not exists: {}", opt.target.display());
    }

    let files = walk_dir(opt.target)?;

    let it = mimes(opt.method, files);
    pin_mut!(it);

    while let Some(data) = it.next().await {
        let (file_path, mime_str) = data?;
        println!("{}: {}", file_path.display(), mime_str.unwrap_or("(none)"));
    }

    Ok(())
}

fn walk_dir(target: PathBuf) -> Fallible<Vec<PathBuf>> {
    use std::fs::read_dir;

    let mut dirs = vec![target];
    let mut files = vec![];

    while let Some(target) = dirs.pop() {
        for dir_entry in read_dir(target)? {
            let dir_entry = match dir_entry {
                Ok(data) => data,
                Err(e) => return Err(e).context("failed to read entry"),
            };

            let entry_path = dir_entry.path();

            if entry_path.is_file() {
                files.push(entry_path);
            } else {
                dirs.push(entry_path);
            }
        }
    }

    Ok(files)
}

fn mimes(
    mime_method: MimeMethod,
    files: Vec<PathBuf>,
) -> impl Stream<Item = Fallible<(PathBuf, Option<&'static str>)>> {
    use tokio::fs;
    use tokio::io;

    let mut handles = FuturesOrdered::new();

    // lower than `ulimit -n`.
    let semaphore = Arc::new(Semaphore::new(64));

    for file in files {
        let mime_method = mime_method.clone();
        let semaphore = semaphore.clone();
        let handle = tokio::task::spawn(async move {
            let buf = {
                let _permit = semaphore.acquire().await?;
                let mut reader = io::BufReader::new(fs::File::open(&file).await?);

                let mut buf = vec![];
                reader.read_to_end(&mut buf).await?;
                buf
            };

            let mime = match mime_method {
                MimeMethod::Infer => infer::get(&buf).map(|data| data.mime_type()),
                #[cfg(feature = "tree_magic_mini")]
                MimeMethod::TreeMagic => Some(tree_magic_mini::from_u8(&buf)),
            };
            Ok::<_, anyhow::Error>((file, mime))
        });

        handles.push_back(handle);
    }

    try_stream! {
        while let Some(ret) = handles.next().await {
            yield ret??;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

    #[ignore]
    #[test]
    fn struct_opt_help() {
        Opt::command().print_help().unwrap();
    }

    #[tokio::test]
    async fn check_target_file() {
        let it = mimes(MimeMethod::Infer, vec![PathBuf::from("./Cargo.toml")]);
        pin_mut!(it);

        while let Some(data) = it.next().await {
            let _data = data.unwrap();
            // dbg!(data);
        }
    }

    #[tokio::test]
    async fn mime_method_infer() {
        let it = mimes(MimeMethod::Infer, walk_dir(PathBuf::from("./src")).unwrap());
        pin_mut!(it);

        while let Some(data) = it.next().await {
            let _data = data.unwrap();
            // dbg!(data);
        }
    }

    #[cfg(feature = "tree_magic_mini")]
    #[tokio::test]
    async fn mime_method_tree_magic() {
        let it = mimes(
            MimeMethod::TreeMagic,
            walk_dir(PathBuf::from("./src")).unwrap(),
        );
        pin_mut!(it);

        while let Some(data) = it.next().await {
            let _data = data.unwrap();
            // dbg!(data);
        }
    }
}

use blake2::{Blake2b512, Digest};
use clap::{ArgGroup, Parser};
use futures::prelude::*;
use rust_myscript::prelude::*;
use std::collections::HashSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::exit;
use tokio::io::AsyncReadExt;
use tracing::{debug, info, warn, Instrument};

#[derive(Parser)]
#[command(group = ArgGroup::new("backup").required(true))]
struct Opt {
    /// Do not write anything, just show what would be done
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Specifies the number of jobs to run simultaneously
    #[arg(short, long)]
    jobs: Option<usize>,

    /// Target directory to deduplicate
    #[arg(short, long)]
    target_dir: Vec<PathBuf>,

    /// Backup destination
    #[arg(short, long, group = "backup")]
    backup_dir: Option<PathBuf>,

    /// Override file without backup
    #[arg(short, long, group = "backup")]
    force: bool,
}

#[cfg(target_os = "macos")]
extern "C" {
    static errno: libc::c_int;

    fn clonefile(
        src: *const libc::c_schar,
        dst: *const libc::c_schar,
        flag: libc::c_int,
    ) -> libc::c_int;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if cfg!(not(target_os = "macos")) {
        eprintln!("need to run on macOS");
        exit(1);
    }

    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt: Opt = Opt::parse();

    if opt.backup_dir.is_none() && !opt.force {
        eprintln!("need to set backup directory or use force flag");
        exit(1);
    }

    debug!(target_dir = ?opt.target_dir);

    let mut files = HashSet::new();
    for target in &opt.target_dir {
        files.extend(walk_dir(target).await?);
    }

    // launchctl limit maxfiles
    let job_num = opt.jobs.unwrap_or_else(num_cpus::get).min(200);

    let mut files = files.into_iter().collect::<Vec<_>>();
    let window = files.len() / job_num;
    let mut futs = futures::stream::FuturesUnordered::new();
    for index in 0..job_num {
        let entries = if index == job_num - 1 {
            files.drain(..).collect::<Vec<_>>()
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
                            Ok(n) if n == 0 => break,
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
            .instrument(tracing::info_span!("fut", index)),
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
            if let Some(ref backup_dir_root) = opt.backup_dir {
                let backup_path = if file_path.has_root() {
                    backup_dir_root.join(file_path.strip_prefix("/")?)
                } else {
                    backup_dir_root.join(file_path)
                };

                let backup_path_parent = backup_path.parent().context("parent")?;

                if opt.dry_run {
                    eprintln!("Would create dir to {backup_path_parent:?}");
                    eprintln!(
                        "Would move to {backup_path_parent:?} from {file_path:?}"
                    );
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
                if opt.dry_run {
                    eprintln!("Would remove {file_path:?}");
                } else {
                    debug!(?file_path, "remove");
                    let ret_rm = tokio::fs::remove_file(file_path).await;
                    if ret_rm.is_err() {
                        eprintln!("failed to remove file: {ret_rm:?}");
                    }
                }
            }

            if opt.dry_run {
                eprintln!("Would clone {source:?} to {file_path:?}");
            } else {
                println!("clone {source:?} to {file_path:?}");

                #[cfg(target_os = "macos")]
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

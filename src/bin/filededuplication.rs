use blake2::{Blake2b, Digest};
use futures::prelude::*;
use log::{debug, info};
use rust_myscript::prelude::*;
use std::collections::HashSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::exit;
use std::sync::Arc;
use structopt::clap::ArgGroup;
use structopt::StructOpt;
use tokio::io::AsyncReadExt;

struct HexFormat<'a>(&'a [u8]);

impl<'a> std::fmt::Display for HexFormat<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        let ret_first = write!(f, "{:X?}", self.0[0]);
        if ret_first.is_err() {
            return ret_first;
        }

        for entry in &self.0[1..self.0.len()] {
            let entry_ret = write!(f, ":{:X?}", entry);
            if entry_ret.is_err() {
                return entry_ret;
            }
        }

        Ok(())
    }
}

#[derive(StructOpt)]
#[structopt(group = ArgGroup::with_name("backup").required(true))]
struct Opt {
    /// Do not write anything, just show what would be done
    #[structopt(short = "n", long)]
    dry_run: bool,

    /// Specifies the number of jobs to run simultaneously
    #[structopt(short, long)]
    jobs: Option<usize>,

    /// Target directory to deduplicate
    #[structopt(short, long, parse(from_os_str))]
    target_dir: Vec<PathBuf>,

    /// Backup destination
    #[structopt(short, long, parse(from_os_str), group = "backup")]
    backup_dir: Option<PathBuf>,

    /// Override file without backup
    #[structopt(short, long, group = "backup")]
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
    env_logger::init();

    info!("Hello");

    let opt: Opt = Opt::from_args();

    if opt.backup_dir.is_none() && !opt.force {
        eprintln!("need to set backup directory or use force flag");
        exit(1);
    }

    debug!("target_dir: {:?}", opt.target_dir);

    let mut files = HashSet::new();
    for target in &opt.target_dir {
        files.extend(walk_dir(target).await?);
    }

    // launchctl limit maxfiles
    let semapho = Arc::new(tokio::sync::Semaphore::new(
        opt.jobs.unwrap_or_else(num_cpus::get).min(200),
    ));
    let mut futs = futures::stream::FuturesUnordered::new();

    for entry in files {
        let semapho = semapho.clone();
        let fut = async move {
            let _lock = semapho.acquire().await;

            info!("calculate begin. path: {:?}", entry);
            let mut digest = Blake2b::new();
            let source_file = tokio::fs::File::open(&entry).await.unwrap();
            let mut reader = tokio::io::BufReader::new(source_file);

            let mut buf = [0u8; 4096];
            loop {
                let n = match reader.read(&mut buf).await {
                    Ok(n) if n == 0 => break,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("{:?}", e);
                        return None;
                    }
                };
                digest.update(&buf[0..n]);
            }

            let hash = digest.finalize_reset().to_vec();
            info!(
                "calculate end. path: {:?}, hash: {}",
                entry,
                HexFormat(&hash)
            );
            Some((entry, hash))
        };
        futs.push(fut);
    }

    let mut file_hash_map = std::collections::HashMap::<Vec<u8>, Vec<PathBuf>>::new();
    while let Some(data) = futs.next().await {
        let (entry, hash) = match data {
            Some(d) => d,
            None => continue,
        };

        match file_hash_map.get_mut(&hash) {
            Some(data) => data.push(entry),
            None => {
                file_hash_map.insert(hash, vec![entry]);
            }
        }
    }

    for value in file_hash_map.values() {
        if value.len() < 2 {
            continue;
        }

        info!("{:?}", value);
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
                    eprintln!("Would create dir to {:?}", backup_path_parent);
                    eprintln!(
                        "Would move to {:?} from {:?}",
                        backup_path_parent, file_path
                    );
                } else {
                    debug!("create {:?}", backup_path_parent);
                    let create_dir_ret = tokio::fs::create_dir_all(backup_path_parent).await;

                    if create_dir_ret.is_err() {
                        eprintln!("failed to create dir: {:?}", create_dir_ret);
                        continue;
                    }

                    debug!("rename to: {:?}, from: {:?}", backup_path, file_path);
                    let move_ret = tokio::fs::rename(&file_path, &backup_path).await;
                    if move_ret.is_err() {
                        eprintln!("failed to move file: {:?}", move_ret);
                        continue;
                    }
                }
            } else {
                // TODO: use repository.
                #[allow(clippy::collapsible_if)]
                if opt.dry_run {
                    eprintln!("Would remove {:?}", file_path);
                } else {
                    debug!("remove: {:?}", file_path);
                    let ret_rm = tokio::fs::remove_file(file_path).await;
                    if ret_rm.is_err() {
                        eprintln!("failed to remove file: {:?}", ret_rm);
                    }
                }
            }

            if opt.dry_run {
                eprintln!("Would clone {:?} to {:?}", source, file_path);
            } else {
                println!("clone {:?} to {:?}", source, file_path);

                #[cfg(target_os = "macos")]
                unsafe {
                    let ret_clonefile = clonefile(
                        CString::new(source.to_str().context("source")?)?.as_ptr(),
                        CString::new(file_path.to_str().context("file_path")?)?.as_ptr(),
                        0,
                    );
                    if ret_clonefile != 0 {
                        eprintln!("failed to execute the clonefile command: {}", errno);
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
                    eprintln!("failed to read entry: {:?}", e);
                    continue;
                }
            };

            let dir_entry_path = dir_entry.path();
            let symlink_meta = tokio::fs::symlink_metadata(&dir_entry_path).await?;
            let symlink_file_type = symlink_meta.file_type();

            if symlink_file_type.is_symlink() {
                debug!("ignore symlink: {:?}", dir_entry_path);
            } else if symlink_file_type.is_dir() {
                debug!("dir: {:?}", dir_entry_path);
                let ret = walk_dir(&dir_entry_path).await?;
                files.extend(ret);
            } else if symlink_file_type.is_file() {
                debug!("file: {:?}", dir_entry_path);
                if symlink_meta.permissions().readonly() {
                    debug!("readonly: {:?}", dir_entry_path);
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

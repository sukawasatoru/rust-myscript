use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::exit;

use blake2::{Blake2b, Digest};
use futures::prelude::*;
use log::{debug, info};
use structopt::StructOpt;
use tokio::prelude::*;

use rust_myscript::myscript::prelude::*;

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
struct Opt {
    /// Target directory to deduplicate
    #[structopt(short, long, parse(from_os_str))]
    target_dir: PathBuf,

    /// Backup destination
    #[structopt(short, long, parse(from_os_str))]
    backup_dir: Option<PathBuf>,

    /// Override file without backup
    #[structopt(short, long)]
    force: bool,
}

extern "C" {
    static errno: libc::c_int;

    fn clonefile(
        src: *const libc::c_schar,
        dst: *const libc::c_schar,
        flag: libc::c_int,
    ) -> libc::c_int;
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let opt: Opt = Opt::from_args();

    if opt.backup_dir.is_none() && !opt.force {
        eprintln!("need to set backup directory or use force flag");
        exit(1);
    }

    if let Some(ref backup_dir) = opt.backup_dir {
        if backup_dir == &opt.target_dir {
            eprintln!("cannot specify same target and backup path");
            exit(1);
        }
    }

    debug!("target_dir: {:?}", opt.target_dir);

    let files: Vec<PathBuf> = walk_dir(&opt.target_dir).await?;

    let mut digest = Blake2b::new();
    let mut file_hash_map = std::collections::HashMap::<Vec<u8>, Vec<PathBuf>>::new();

    'file_entry: for entry in files {
        info!("calculate begin. path: {:?}", entry);
        let source_file: tokio::fs::File = tokio::fs::File::open(&entry).await?;
        let mut reader = tokio::io::BufReader::new(source_file);

        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf).await {
                Ok(n) if n == 0 => break,
                Ok(n) => n,
                Err(e) => {
                    eprintln!("{:?}", e);
                    continue 'file_entry;
                }
            };
            digest.input(&buf[0..n]);
        }

        let hash = digest.result_reset().to_vec();
        info!(
            "calculate end. path: {:?}, hash: {}",
            entry,
            HexFormat(&hash)
        );
        match file_hash_map.get_mut(&hash) {
            Some(data) => data.push(entry),
            None => {
                file_hash_map.insert(hash, vec![entry]);
            }
        }
    }

    for (key, value) in file_hash_map {
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

                let create_dir_ret =
                    tokio::fs::create_dir_all(&backup_path.parent().ok_or_err()?).await;

                if create_dir_ret.is_err() {
                    eprintln!("failed to create dir: {:?}", create_dir_ret);
                    continue;
                }

                let move_ret = tokio::fs::rename(&file_path, &backup_path).await;
                if move_ret.is_err() {
                    eprintln!("failed to move file: {:?}", move_ret);
                    continue;
                }
            } else {
                let ret_rm = tokio::fs::remove_file(file_path).await;
                if ret_rm.is_err() {
                    eprintln!("failed to remove file: {:?}", ret_rm);
                }
            }

            println!("clone {:?} to {:?}", source, file_path);
            unsafe {
                let ret_clonefile = clonefile(
                    CString::new(source.to_str().ok_or_err()?)?.as_ptr(),
                    CString::new(file_path.to_str().ok_or_err()?)?.as_ptr(),
                    0,
                );
                if ret_clonefile != 0 {
                    eprintln!("failed to execute the clonefile command: {}", errno);
                }
            };
        }
    }

    info!("Bye");

    Ok(())
}

fn walk_dir(target_dir: &Path) -> Pin<Box<dyn '_ + Future<Output = Fallible<Vec<PathBuf>>>>> {
    Box::pin(async move {
        let mut read_dir: tokio::fs::ReadDir = tokio::fs::read_dir(target_dir).await?;
        let mut files = Vec::<PathBuf>::new();
        loop {
            let dir_entry: tokio::fs::DirEntry = match read_dir.next_entry().await {
                Ok(None) => break,
                Ok(Some(data)) => data,
                Err(e) => {
                    eprintln!("failed to read entry: {:?}", e);
                    continue;
                }
            };

            let dir_entry_path: PathBuf = dir_entry.path();
            let symlink_meta: std::fs::Metadata =
                tokio::fs::symlink_metadata(&dir_entry_path).await?;
            let symlink_file_type = symlink_meta.file_type();

            if symlink_file_type.is_symlink() {
                debug!("ignore symlink: {:?}", dir_entry_path);
            } else if symlink_file_type.is_dir() {
                debug!("dir: {:?}", dir_entry_path);
                let mut ret = walk_dir(&dir_entry_path).await?;
                files.append(&mut ret);
            } else if symlink_file_type.is_file() {
                debug!("file: {:?}", dir_entry_path);
                if symlink_meta.permissions().readonly() {
                    debug!("readonly: {:?}", dir_entry_path);
                    continue;
                }
                files.push(dir_entry_path);
            } else {
                unreachable!();
            }
        }

        Ok(files)
    })
}

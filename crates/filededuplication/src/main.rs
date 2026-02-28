/*
 * Copyright 2020, 2021, 2022, 2023, 2024, 2025, 2026 sukawasatoru
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

use clap::{ArgGroup, Parser, Subcommand};
use filededuplication::feature::remove_file::remove_file;
use rust_myscript::prelude::*;
use std::path::PathBuf;
use tracing_subscriber::filter::LevelFilter;

#[derive(Parser)]
struct Opt {
    /// Verbose mode (-v, -vv, -vvv, etc.)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Use clonefile to deduplicate files (only macOS)
    #[command(group = ArgGroup::new("backup").required(true))]
    Clone {
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
    },
    /// Compare and remove files that are the same as the master directory files
    Remove {
        /// Do not write anything, just show what would be done
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Specifies the number of jobs to run simultaneously
        #[arg(short, long)]
        jobs: Option<usize>,

        /// Master directory to compare
        #[arg(long)]
        master: PathBuf,

        /// Target directory to compare and remove files if the master directory contains the same
        /// file
        #[arg(long)]
        shrink: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let opt: Opt = Opt::parse();

    tracing_subscriber::fmt()
        .with_max_level(match opt.verbose {
            0 => LevelFilter::INFO,
            1 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        })
        .init();

    match opt.cmd {
        Command::Clone {
            dry_run,
            jobs,
            target_dir,
            backup_dir,
            force,
        } => {
            #[cfg(target_os = "macos")]
            {
                filededuplication::feature::clone_file(dry_run, jobs, target_dir, backup_dir, force)
                    .await
            }

            #[cfg(not(target_os = "macos"))]
            {
                bail!(
                    "clonefile is not supported on this platform: {}",
                    std::env::consts::OS
                );
            }
        }
        Command::Remove {
            dry_run,
            jobs,
            master,
            shrink,
        } => remove_file(dry_run, jobs, master, shrink).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert()
    }
}

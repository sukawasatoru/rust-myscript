/*
 * Copyright 2023 sukawasatoru
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

use clap::Parser;
use rust_myscript::prelude::*;
use std::path::PathBuf;
use tracing::trace;

/// The tar like command wrapper for rar.
#[derive(Parser)]
struct Opt {
    /// '-C' of tar
    #[arg(short = 'C', long, alias = "directory", value_hint = clap::ValueHint::DirPath)]
    cd: Option<PathBuf>,

    /// '-c' of tar
    #[arg(short, long, group = "mode")]
    create: bool,

    /// '-f' of tar
    #[arg(short)]
    f: String,

    /// '-x' of tar
    #[arg(short, group = "mode")]
    x: bool,

    /// '-v' for tar
    #[arg(short)]
    v: bool,

    /// '-t' of tar
    #[arg(short = 't', long)]
    list: bool,

    /// Compression method (0..=5).
    #[arg(short, value_parser = clap::value_parser!(u8).range(0..=5))]
    m: Option<u8>,

    /// Add data recovery record.
    #[arg(value_name = "n", long)]
    rr: Option<u16>,

    /// Encrypt both file data and header.
    #[arg(long)]
    hp: bool,

    // should separate for e.g. '-cvf'.
    /// verbose
    #[arg(long)]
    verbose: bool,

    targets: Vec<String>,
}

fn main() -> Fallible<()> {
    check_rar()?;

    let opt = Opt::parse();
    setup_log(opt.verbose);

    let mut args = vec![];

    if opt.create {
        args.push("a");
    } else if opt.x {
        args.push("e");
    }

    if opt.list {
        if opt.v {
            args.push("lt");
        } else {
            args.push("t");
        }
    }

    let m = opt.m.map(|data| format!("-m{}", data));
    if let Some(ref data) = m {
        args.extend_from_slice(&[data]);
    }

    let rr = opt.rr.map(|data| data.to_string());
    if let Some(ref n) = rr {
        args.extend_from_slice(&["-rr", n]);
    }

    if opt.hp {
        args.push("-hp");
    }

    args.push(&opt.f);

    let cd = opt.cd.map(|data| format!("{}/", data.display()));
    if let Some(ref cd) = cd {
        args.push(cd);
    }

    args.extend_from_slice(&opt.targets.iter().map(AsRef::as_ref).collect::<Vec<_>>());

    trace!(?args);

    let ret = std::process::Command::new("rar")
        .args(&args)
        .spawn()?
        .wait()?
        .code();

    check_code(ret)?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn check_rar() -> Fallible<()> {
    let ret = std::process::Command::new("type")
        .arg("rar")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?
        .wait()?;
    if !ret.success() {
        bail!("rar not found")
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn check_rar() -> Fallible<()> {
    bail!("not supported yet")
}

fn check_code(code: Option<i32>) -> Fallible<()> {
    let Some(code) = code else {
        bail!("Process terminated by signal")
    };

    match code {
        0 => Ok(()),
        1 => bail!("Non fatal error(s) occurred."),
        2 => bail!("A fatal error occurred."),
        3 => bail!("Invalid checksum. Data is damaged."),
        4 => bail!("Attempt to modify an archive locked by 'k' command."),
        5 => bail!("Write error."),
        6 => bail!("File open error."),
        7 => bail!("Wrong command line option."),
        8 => bail!("Not enough memory."),
        9 => bail!("File create error"),
        10 => bail!("No files matching the specified mask and options were found."),
        11 => bail!("Wrong password."),
        12 => bail!("Read error."),
        255 => bail!("User stopped the process."),
        _ => bail!("unexpected code: {}", code),
    }
}

fn setup_log(level: bool) {
    use tracing_subscriber::filter::LevelFilter;

    tracing_subscriber::fmt()
        .with_max_level(match level {
            false => LevelFilter::OFF,
            true => LevelFilter::TRACE,
        })
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn opt() {
        Opt::command().debug_assert();
    }

    #[ignore]
    #[test]
    fn opt_help() {
        Opt::command().print_help().unwrap();
    }
}

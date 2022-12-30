/*
 * Copyright 2022 sukawasatoru
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
use futures::StreamExt;
use rust_myscript::prelude::*;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info};

/// Check new crate from specified Cargo.toml.
#[derive(Parser)]
struct Opt {
    /// Doesn't update 'crates.io-index.git' repository before check crate versions.
    #[arg(long)]
    no_fetch: bool,

    /// Includes prerelease version.
    #[arg(long)]
    pre_release: bool,

    /// A 'Cargo.toml' to check crate.
    cargo_file: PathBuf,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt = Opt::parse();

    check_git()?;

    if !opt.cargo_file.exists() {
        bail!("{} is not exists", opt.cargo_file.display())
    }

    let crates = read_crates(&opt.cargo_file)?;

    let project_dirs = directories::ProjectDirs::from("com", "sukawasatoru", "Crate Updater")
        .expect("no valid home directory");
    let cache_dir = project_dirs.cache_dir();

    debug!(?cache_dir);

    if !cache_dir.exists() {
        create_dir_all(cache_dir)?;
    }

    let repo_path = cache_dir.join("crates.io-index");
    if repo_path.exists() {
        if opt.no_fetch {
            debug!("skip fetch");
        } else {
            git_fetch(&repo_path)?;
        }
    } else {
        git_clone(cache_dir)?;
    }

    let mut futs = futures::stream::FuturesOrdered::new();
    let repo_path = Arc::new(repo_path);
    for (crate_name, current_version) in crates {
        let repo_path = repo_path.clone();
        futs.push_back(tokio::spawn(async move {
            let ret = read_latest_version(&repo_path, &crate_name, opt.pre_release);
            (crate_name, current_version, ret)
        }));
    }

    while let Some(data) = futs.next().await {
        let (crate_name, current_version, latest_version) = data?;
        let latest_version = match latest_version {
            Ok(data) => data,
            Err(e) => {
                info!(?e);
                eprintln!("failed to check crate version");
                continue;
            }
        };
        if current_version < latest_version {
            println!(
                "name: {}, current: {}, latest: {}",
                crate_name, current_version, latest_version
            );
        }
    }
    info!("Bye");

    Ok(())
}

#[derive(Deserialize)]
struct CargoFile {
    dependencies: HashMap<String, CargoDependencyEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CargoDependencyEntry {
    String(#[serde(deserialize_with = "deserialize_semver")] semver::Version),
    Table(CargoDependencyTableEntry),
    Unsupported(toml::Value),
}

#[derive(Debug, Deserialize)]
struct CargoDependencyTableEntry {
    #[serde(deserialize_with = "deserialize_semver_option")]
    version: Option<semver::Version>,
}

fn check_git() -> Fallible<()> {
    let ret = std::process::Command::new("git")
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?
        .wait()?;
    if ret.code().context("terminated")? != 0 {
        bail!("git not found");
    }

    Ok(())
}

fn git_fetch(repo_path: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .arg("fetch")
        .current_dir(repo_path)
        .spawn()?
        .wait()?;

    match status_code.code() {
        Some(0) => Ok(()),
        Some(_) => {
            bail!("failed to fetch repository: {}", status_code)
        }
        None => bail!("killed git process"),
    }
}

fn git_clone(cache_dir: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .args([
            "clone",
            "--filter=blob:none",
            "https://github.com/rust-lang/crates.io-index.git",
        ])
        .current_dir(cache_dir)
        .spawn()?
        .wait()?;

    match status_code.code() {
        Some(0) => Ok(()),
        Some(_) => {
            bail!("failed to clone repository: {}", status_code)
        }
        None => bail!("killed git process"),
    }
}

fn read_crates(file_path: &Path) -> Fallible<Vec<(String, semver::Version)>> {
    let mut toml_string = String::new();
    let mut buf = BufReader::new(File::open(file_path)?);
    buf.read_to_string(&mut toml_string)?;

    let cargo_file = toml::from_str::<CargoFile>(&toml_string)?;

    let mut crates = vec![];
    for (key, value) in cargo_file.dependencies.into_iter() {
        match value {
            CargoDependencyEntry::String(data) => {
                crates.push((key, data));
            }
            CargoDependencyEntry::Table(CargoDependencyTableEntry {
                version: Some(data),
            }) => {
                crates.push((key, data));
            }
            _ => {
                debug!(%key, ?value, "unexpected version structure");
            }
        }
    }

    Ok(crates)
}

fn read_latest_version(
    repo_path: &Path,
    crate_name: &str,
    pre_release: bool,
) -> Fallible<semver::Version> {
    let git_result = std::process::Command::new("git")
        .args(["ls-files", "-z", &format!("*/{}", crate_name)])
        .stdout(std::process::Stdio::piped())
        .current_dir(repo_path)
        .spawn()?
        .wait_with_output()?;
    let stdout = String::from_utf8(git_result.stdout).context("failed to convert stdout")?;
    debug!(%crate_name, %stdout);

    match git_result.status.code() {
        Some(0) => {}
        Some(_) => bail!("failed to find version file: {}", git_result.status),
        None => bail!("killed git process"),
    }

    let file_path = repo_path.join(stdout.trim_end_matches('\0'));
    let mut file_string = String::new();
    let mut buf = BufReader::new(File::open(file_path)?);

    let mut latest = semver::Version::new(0, 0, 0);
    loop {
        file_string.clear();
        let ret = buf.read_line(&mut file_string);
        match ret {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(e).context("failed to read version file"),
        }

        let line_version = serde_json::from_str::<CratesIOVersion>(&file_string)?;
        if !line_version.vers.pre.is_empty() && !pre_release {
            continue;
        }
        if latest < line_version.vers {
            latest = line_version.vers;
        }
    }

    Ok(latest)
}

fn deserialize_semver_option<'de, D>(de: D) -> Result<Option<semver::Version>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionVisitor;
    impl<'de> Visitor<'de> for OptionVisitor {
        type Value = Option<semver::Version>;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            formatter.write_str("an version string")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            Ok(Some(deserialize_semver(deserializer)?))
        }
    }

    de.deserialize_option(OptionVisitor)
}

fn deserialize_semver<'de, D>(de: D) -> Result<semver::Version, D::Error>
where
    D: Deserializer<'de>,
{
    struct VersionString;
    impl<'de> Visitor<'de> for VersionString {
        type Value = semver::Version;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            formatter.write_str("an version string")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            semver::Version::parse(v.trim_start_matches('=')).map_err(de::Error::custom)
        }
    }

    de.deserialize_string(VersionString)
}

#[derive(Deserialize)]
struct CratesIOVersion {
    vers: semver::Version,
}

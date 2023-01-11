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

use chrono::{DateTime, Duration, TimeZone, Utc};
use clap::{CommandFactory, Parser, ValueHint};
use futures::StreamExt;
use rust_myscript::prelude::*;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info};

/// Check new crate from specified Cargo.toml.
#[derive(Parser)]
#[clap(name = "crate-checker", group = clap::ArgGroup::new("fetch").multiple(false))]
struct Opt {
    /// Doesn't update 'crates.io-index.git' repository before check crate versions.
    #[arg(long, group = "fetch")]
    no_fetch: bool,

    /// Update 'crates.io-index.git' always.
    #[arg(long, group = "fetch")]
    force_fetch: bool,

    /// Includes prerelease version.
    #[arg(long)]
    pre_release: bool,

    /// Generate shell completions.
    #[arg(long, exclusive = true)]
    completion: Option<clap_complete::Shell>,

    /// A 'Cargo.toml' to check crate.
    #[clap(value_hint = ValueHint::FilePath, required_unless_present = "completion")]
    cargo_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt = Opt::parse();

    if let Some(data) = opt.completion {
        clap_complete::generate(
            data,
            &mut Opt::command(),
            "crate-checker",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    check_git()?;

    let cargo_file = opt.cargo_file.expect("required_unless_present");
    if !cargo_file.exists() {
        bail!("{} is not exists", cargo_file.display())
    }

    let current_time = Utc::now();
    let project_dirs = directories::ProjectDirs::from("com", "sukawasatoru", "Crate Updater")
        .expect("no valid home directory");
    let prefs_path = project_dirs.config_dir().join("preferences.toml");

    let mut prefs = load_prefs(&prefs_path)?;

    let crates = read_crates(&cargo_file)?;

    let cache_dir = project_dirs.cache_dir();

    debug!(?cache_dir);

    if !cache_dir.exists() {
        create_dir_all(cache_dir)?;
    }

    let repo_path = cache_dir.join("crates.io-index");
    if repo_path.exists() {
        if opt.no_fetch {
            debug!("skip fetch");
        } else if opt.force_fetch || Duration::minutes(5) < current_time - prefs.last_fetch {
            debug!("fetch");
            git_pull(&repo_path)?;
            prefs.last_fetch = current_time;
            store_prefs(&prefs_path, &prefs)?;
        }
    } else {
        git_clone(cache_dir)?;
        prefs.last_fetch = current_time;
        store_prefs(&prefs_path, &prefs)?;
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

fn git_pull(repo_path: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .arg("pull")
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
        if line_version.yanked || (!line_version.vers.pre.is_empty() && !pre_release) {
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
    yanked: bool,
}

#[derive(Deserialize, Serialize)]
struct Prefs {
    last_fetch: DateTime<Utc>,
}

fn load_prefs(prefs_path: &Path) -> Fallible<Prefs> {
    let config_path = prefs_path.parent().context("config directory")?;

    if !config_path.exists() {
        create_dir_all(config_path)?;
    }

    if !prefs_path.exists() {
        return Ok(Prefs {
            last_fetch: Utc.timestamp_opt(0, 0).unwrap(),
        });
    }

    let mut prefs_string = String::new();
    let mut buf = BufReader::new(File::open(prefs_path)?);

    buf.read_to_string(&mut prefs_string)?;
    Ok(toml::from_str(&prefs_string)?)
}

fn store_prefs(prefs_path: &Path, prefs: &Prefs) -> Fallible<()> {
    let config_path = prefs_path.parent().context("config directory")?;

    if !config_path.exists() {
        create_dir_all(config_path)?;
    }

    let mut buf = BufWriter::new(File::create(prefs_path)?);
    buf.write_all(toml::to_string(prefs)?.as_bytes())?;
    buf.flush()?;

    Ok(())
}

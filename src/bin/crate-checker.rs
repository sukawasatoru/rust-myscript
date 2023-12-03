/*
 * Copyright 2022, 2023 sukawasatoru
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

use chrono::{DateTime, TimeZone, Utc};
use clap::{CommandFactory, Parser, ValueHint};
use futures::StreamExt;
use reqwest::StatusCode;
use rust_myscript::prelude::*;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Formatter;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use url::Url;

/// Check new crate from specified Cargo.toml.
#[derive(Parser)]
#[clap(name = "crate-checker", group = clap::ArgGroup::new("fetch").multiple(false))]
struct Opt {
    /// Ignored for compatibility with older implementations.
    #[arg(long, group = "fetch")]
    #[deprecated = "ignored for compatibility with older implementations"]
    no_fetch: bool,

    /// Request 'index.creates.io' always.
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

    let cargo_file = opt.cargo_file.expect("required_unless_present");
    if !cargo_file.exists() {
        bail!("{} is not exists", cargo_file.display())
    }

    let project_dirs = directories::ProjectDirs::from("com", "sukawasatoru", "Crate Updater")
        .expect("no valid home directory");

    let crates = read_crates(&cargo_file)?;

    let cache_dir = project_dirs.cache_dir();

    debug!(?cache_dir);

    if !cache_dir.exists() {
        create_dir_all(cache_dir)?;
    }

    let cache_dir_sparse = Arc::new(cache_dir.join("sparse"));
    if !cache_dir_sparse.exists() {
        create_dir_all(&*cache_dir_sparse)?;
    }

    let client = reqwest::Client::builder()
        .user_agent("crate-checker")
        .build()?;

    // TODO: delete repo.
    let repo_path = cache_dir.join("crates.io-index");

    let mut futs = futures::stream::FuturesOrdered::new();
    let semaphore = Arc::new(Semaphore::new(8));
    for (crate_name, current_version) in crates {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let cache_dir_sparse = cache_dir_sparse.clone();
        futs.push_back(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let ret =
                fetch_latest_version(client, &cache_dir_sparse, &crate_name, opt.pre_release).await;
            (crate_name, current_version, ret)
        }));
    }

    let mut updated_map = BTreeMap::<String, (semver::Version, semver::Version)>::new();
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
            updated_map.insert(crate_name, (current_version, latest_version));
        }
    }

    for (crate_name, (current_version, latest_version)) in updated_map {
        println!("name: {crate_name}, current: {current_version}, latest: {latest_version}");
    }
    info!("Bye");

    Ok(())
}

#[derive(Deserialize)]
struct CargoFile {
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<HashMap<String, CargoDependencyEntry>>,
    dependencies: HashMap<String, CargoDependencyEntry>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<HashMap<String, CargoDependencyEntry>>,
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

fn read_crates(file_path: &Path) -> Fallible<Vec<(String, semver::Version)>> {
    use std::fs::File;
    use std::io::{BufReader, Read};

    let mut toml_string = String::new();
    let mut buf = BufReader::new(File::open(file_path)?);
    buf.read_to_string(&mut toml_string)?;

    let cargo_file = toml::from_str::<CargoFile>(&toml_string)?;

    let crates = [
        cargo_file.build_dependencies.unwrap_or_default(),
        cargo_file.dependencies,
        cargo_file.dev_dependencies.unwrap_or_default(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|(key, value)| match value {
        CargoDependencyEntry::String(data) => Some((key, data)),
        CargoDependencyEntry::Table(CargoDependencyTableEntry {
            version: Some(data),
        }) => Some((key, data)),
        _ => {
            debug!(%key, ?value, "unexpected version structure");
            None
        }
    })
    .collect::<Vec<_>>();

    Ok(crates)
}

#[tracing::instrument(skip(client, cache_dir, pre_release))]
async fn fetch_latest_version(
    client: reqwest::Client,
    cache_dir: &Path,
    crate_name: &str,
    pre_release: bool,
) -> Fallible<semver::Version> {
    let target = Url::parse("https://index.crates.io")?.join(&create_crate_path(crate_name))?;

    let builder = client.get(target);

    async fn request_get(builder: reqwest::RequestBuilder) -> Fallible<reqwest::Response> {
        let res = builder.send().await?;
        res.error_for_status_ref()
            .context("server returned an error")?;
        Ok(res)
    }

    let retrieve_and_store_text = |res: reqwest::Response| async move {
        let etag = res
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|data| match data.to_str() {
                Ok(etag) => Some(etag.to_string()),
                Err(e) => {
                    warn!("failed to convert etag to text: {crate_name}, {:?}", e);
                    None
                }
            });

        debug!("retrieve body");
        let text = res.text().await?;

        if let Some(etag) = etag {
            if let Err(e) = store_cache(cache_dir, crate_name, &etag, &text).await {
                warn!("failed to store cache: {crate_name}, {:?}", e);
            }
        }

        Result::<String, anyhow::Error>::Ok(text)
    };

    let text = match load_cached_text(cache_dir, crate_name).await {
        Ok(None) => {
            debug!("request");
            let res = request_get(builder).await?;
            retrieve_and_store_text(res).await?
        }
        Ok(Some((etag, text))) => {
            debug!(%etag, "request w/ etag");
            let res = request_get(builder.header(reqwest::header::IF_NONE_MATCH, etag)).await?;

            if res.status() == StatusCode::NOT_MODIFIED {
                debug!("use cache");
                text
            } else {
                retrieve_and_store_text(res).await?
            }
        }
        Err(e) => {
            warn!("request (failed to retrieve cache): {:?}", e);
            let res = request_get(builder).await?;
            retrieve_and_store_text(res).await?
        }
    };

    trace!(%crate_name, %text);

    let mut found = false;
    let mut latest = semver::Version::new(0, 0, 0);
    for line in text.lines() {
        if line.is_empty() {
            debug!("continue");
            continue;
        }

        let line_version = serde_json::from_str::<CratesIOVersion>(line)?;
        if line_version.yanked || (!line_version.vers.pre.is_empty() && !pre_release) {
            continue;
        }
        if latest < line_version.vers {
            found = true;
            latest = line_version.vers;
        }
    }

    if !found {
        bail!("version not found: {crate_name}");
    }

    Ok(latest)
}

#[tracing::instrument(skip(cache_dir))]
async fn load_cached_text(
    cache_dir: &Path,
    crate_name: &str,
) -> Fallible<Option<(String, String)>> {
    let cache_path = cache_dir.join(crate_name);
    let cached_string = match tokio::fs::try_exists(&cache_path).await? {
        true => {
            let mut reader = tokio::io::BufReader::new(tokio::fs::File::open(&cache_path).await?);
            let mut buf = String::new();
            reader.read_to_string(&mut buf).await?;
            buf
        }
        false => return Ok(None),
    };

    let etag_path = cache_dir.join(format!("{crate_name}.etag"));
    let etag = match tokio::fs::try_exists(&etag_path).await? {
        true => {
            let mut reader = tokio::io::BufReader::new(tokio::fs::File::open(&etag_path).await?);
            let mut buf = String::new();
            reader.read_to_string(&mut buf).await?;
            buf
        }
        false => {
            warn!("cache exist but etag not found");
            return Ok(None);
        }
    };

    Ok(Some((etag, cached_string)))
}

async fn store_cache(cache_dir: &Path, crate_name: &str, etag: &str, text: &str) -> Fallible<()> {
    async fn write_to_file(p: &Path, data: &str) -> Fallible<()> {
        let mut writer = tokio::io::BufWriter::new(tokio::fs::File::create(p).await?);
        writer.write_all(data.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    let etag_path = cache_dir.join(format!("{crate_name}.etag"));
    write_to_file(&etag_path, etag).await?;

    let cache_path = cache_dir.join(crate_name);
    write_to_file(&cache_path, text).await?;

    Ok(())
}

/// https://doc.rust-lang.org/cargo/reference/registry-index.html#index-files
fn create_crate_path(name: &str) -> String {
    match name.len() {
        1 => format!("/1/{name}"),
        2 => format!("/2/{name}"),
        3 => format!("/3/{}/{name}", &name[0..1]),
        _ => format!("/{}/{}/{name}", &name[0..=1], &name[2..=3]),
    }
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

#[allow(dead_code)]
fn load_prefs(prefs_path: &Path) -> Fallible<Prefs> {
    use std::fs::File;
    use std::io::{BufReader, Read};

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

#[allow(dead_code)]
fn store_prefs(prefs_path: &Path, prefs: &Prefs) -> Fallible<()> {
    use std::fs::File;
    use std::io::{BufWriter, Write};

    let config_path = prefs_path.parent().context("config directory")?;

    if !config_path.exists() {
        create_dir_all(config_path)?;
    }

    let mut buf = BufWriter::new(File::create(prefs_path)?);
    buf.write_all(toml::to_string(prefs)?.as_bytes())?;
    buf.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_crate_path_1() {
        assert_eq!("/1/a", create_crate_path("a"));
    }

    #[test]
    fn create_crate_path_2() {
        assert_eq!("/2/aa", create_crate_path("aa"));
    }

    #[test]
    fn create_crate_path_3() {
        assert_eq!("/3/a/abc", create_crate_path("abc"));
    }

    #[test]
    fn create_crate_path_4() {
        assert_eq!("/ab/cd/abcd", create_crate_path("abcd"));
    }

    #[test]
    fn create_crate_path_5() {
        assert_eq!("/ab/cd/abcde", create_crate_path("abcde"));
    }
}

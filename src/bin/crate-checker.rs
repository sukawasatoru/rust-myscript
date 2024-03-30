/*
 * Copyright 2022, 2023, 2024 sukawasatoru
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

use chrono::{DateTime, Utc};
use clap::{CommandFactory, Parser, ValueHint};
use futures::StreamExt;
use reqwest::StatusCode;
use rust_myscript::prelude::*;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Formatter;
use std::fs::{create_dir_all, File};
use std::future::Future;
use std::io::BufRead;
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

    let crates = read_crates_from_path(&cargo_file)?;

    let cache_dir = project_dirs.cache_dir();

    debug!(?cache_dir);

    if !cache_dir.exists() {
        create_dir_all(cache_dir)?;
    }

    let cache_dir_sparse = cache_dir.join("sparse");
    if !cache_dir_sparse.exists() {
        create_dir_all(&*cache_dir_sparse)?;
    }

    let repo_path = cache_dir.join("crates.io-index");
    if repo_path.exists() {
        info!("remove caches for git protocol");
        std::fs::remove_dir_all(repo_path)?;
    }

    let client = Arc::new(CratesIOClient::create(
        reqwest::Client::builder()
            .user_agent("crate-checker")
            .build()?,
        CratesCacheFile {
            dir: cache_dir_sparse,
        },
    ));

    let mut futs = futures::stream::FuturesOrdered::new();
    let semaphore = Arc::new(Semaphore::new(8));
    for (crate_name, current_version) in crates {
        let client = client.clone();
        let semaphore = semaphore.clone();
        futs.push_back(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let ret = client
                .fetch_latest_version(&crate_name, opt.pre_release, opt.force_fetch)
                .await;
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

    Ok(())
}

#[derive(Deserialize)]
struct CargoFile {
    #[serde(rename = "build-dependencies", default)]
    build_dependencies: HashMap<String, CargoDependencyEntry>,
    #[serde(default)]
    dependencies: HashMap<String, CargoDependencyEntry>,
    #[serde(rename = "dev-dependencies", default)]
    dev_dependencies: HashMap<String, CargoDependencyEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CargoDependencyEntry {
    String(#[serde(deserialize_with = "deserialize_semver")] semver::Version),
    Table(CargoDependencyTableEntry),
    Unsupported(#[allow(dead_code)] toml::Value),
}

#[derive(Debug, Deserialize)]
struct CargoDependencyTableEntry {
    #[serde(deserialize_with = "deserialize_semver_option")]
    version: Option<semver::Version>,
}

fn read_crates_from_path(file_path: &Path) -> Fallible<Vec<(String, semver::Version)>> {
    read_crates(std::io::BufReader::new(File::open(file_path)?))
}

fn read_crates<R: BufRead>(mut reader: R) -> Fallible<Vec<(String, semver::Version)>> {
    let mut toml_string = String::new();
    reader.read_to_string(&mut toml_string)?;

    let cargo_file = toml::from_str::<CargoFile>(&toml_string)?;

    let crates = [
        cargo_file.build_dependencies,
        cargo_file.dependencies,
        cargo_file.dev_dependencies,
    ]
    .into_iter()
    .flatten()
    .filter_map(|(key, value)| match value {
        CargoDependencyEntry::String(data) => Some((key, data)),
        CargoDependencyEntry::Table(CargoDependencyTableEntry {
            version: Some(data),
        }) => Some((key, data)),
        CargoDependencyEntry::Table(CargoDependencyTableEntry { version: None })
        | CargoDependencyEntry::Unsupported(_) => {
            debug!(%key, ?value, "unexpected version structure");
            None
        }
    })
    .collect::<Vec<_>>();

    Ok(crates)
}

#[derive(Eq, PartialEq, Debug)]
struct ETag(String);

trait CratesCache: Send {
    fn load(&self, name: &str) -> impl Future<Output = Option<(ETag, String)>> + Send;

    fn save(
        &self,
        name: &str,
        etag: &ETag,
        value: &str,
    ) -> impl Future<Output = Fallible<()>> + Send;
}

struct CratesCacheFile {
    dir: PathBuf,
}

impl CratesCache for CratesCacheFile {
    #[tracing::instrument(skip(self))]
    async fn load(&self, name: &str) -> Option<(ETag, String)> {
        let cache_path = self.dir.join(name);

        let exists = tokio::fs::try_exists(&cache_path)
            .await
            .unwrap_or_else(|e| {
                warn!(?e, cache_path = %cache_path.display(), "failed to perform metadata call");
                false
            });

        let cached_string = if exists {
            let mut reader = match tokio::fs::File::open(&cache_path).await {
                Ok(data) => tokio::io::BufReader::new(data),
                Err(e) => {
                    warn!(?e, "failed to open file");
                    return None;
                }
            };

            let mut buf = String::new();
            match reader.read_to_string(&mut buf).await {
                Ok(_) => buf,
                Err(e) => {
                    warn!(?e, cache_path = %cache_path.display(), "read_to_string");
                    return None;
                }
            }
        } else {
            debug!(cache_path = %cache_path.display(), "not found");
            return None;
        };

        let etag_path = self.dir.join(format!("{name}.etag"));
        let exists = tokio::fs::try_exists(&etag_path).await.unwrap_or_else(|e| {
            warn!(?e, etag_path = %etag_path.display(), "failed to perform metadata call");
            false
        });

        let etag = if exists {
            let mut reader = match tokio::fs::File::open(&etag_path).await {
                Ok(data) => tokio::io::BufReader::new(data),
                Err(e) => {
                    warn!(?e, etag_path = %etag_path.display(), "failed to open file");
                    return None;
                }
            };

            let mut buf = String::new();
            match reader.read_to_string(&mut buf).await {
                Ok(_) => ETag(buf),
                Err(e) => {
                    warn!(?e, etag_path = %etag_path.display(), "read_to_string");
                    return None;
                }
            }
        } else {
            debug!(etag_path = %etag_path.display(), "not found");
            return None;
        };

        Some((etag, cached_string))
    }

    async fn save(&self, name: &str, etag: &ETag, value: &str) -> Fallible<()> {
        async fn write_to_file(p: &Path, data: &str) -> Fallible<()> {
            let mut writer = tokio::io::BufWriter::new(tokio::fs::File::create(p).await?);
            writer.write_all(data.as_bytes()).await?;
            writer.flush().await?;
            Ok(())
        }

        write_to_file(&self.dir.join(format!("{name}.etag")), &etag.0).await?;
        write_to_file(&self.dir.join(name), value).await?;

        Ok(())
    }
}

struct CratesIOClient<Cache: CratesCache> {
    client: reqwest::Client,
    base_url: Url,
    cache: Cache,
}

impl<Cache: CratesCache> CratesIOClient<Cache> {
    fn create(client: reqwest::Client, cache: Cache) -> Self {
        Self::create_with_base_url(
            client,
            cache,
            Url::parse("https://index.crates.io").expect("base_url"),
        )
    }

    fn create_with_base_url(client: reqwest::Client, cache: Cache, base_url: Url) -> Self {
        Self {
            client,
            base_url,
            cache,
        }
    }

    #[tracing::instrument(skip(self, pre_release))]
    async fn fetch_latest_version(
        &self,
        crate_name: &str,
        pre_release: bool,
        force: bool,
    ) -> Fallible<semver::Version> {
        let target = self.base_url.join(&create_crate_path(crate_name))?;

        let builder = self.client.get(target);

        async fn request_get(builder: reqwest::RequestBuilder) -> Fallible<reqwest::Response> {
            let res = builder.send().await?;
            res.error_for_status_ref()
                .context("server returned an error")?;
            Ok(res)
        }

        let retrieve_and_store_text = |res: reqwest::Response| async move {
            let etag =
                res.headers()
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
                if let Err(e) = self.cache.save(crate_name, &ETag(etag), &text).await {
                    warn!(?e, crate_name, "failed to store cache");
                }
            }

            Result::<String, anyhow::Error>::Ok(text)
        };

        let text = match self.cache.load(crate_name).await {
            Some((etag, text)) if !force => {
                debug!(etag = %etag.0, "request w/ etag");
                let res =
                    request_get(builder.header(reqwest::header::IF_NONE_MATCH, &etag.0)).await?;

                if res.status() == StatusCode::NOT_MODIFIED {
                    debug!("use cache");
                    text
                } else {
                    retrieve_and_store_text(res).await?
                }
            }
            Some(_) | None => {
                debug!("request");
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

#[cfg(test)]
mod tests {
    use super::*;
    use semver::{BuildMetadata, Prerelease};
    use std::io::{BufReader, BufWriter, Read, Write};

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

    #[test]
    fn read_crates_ok() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[dependencies]
anyhow = "=1.0.80"
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "anyhow".to_string(),
                semver::Version::parse("1.0.80").unwrap()
            )]
        );
    }

    #[test]
    fn read_crates_build_dependencies() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[build-dependencies]
anyhow = "=1.0.80"
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "anyhow".to_string(),
                semver::Version::parse("1.0.80").unwrap()
            )]
        );
    }

    #[test]
    fn read_crates_with_build_dependencies() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[build-dependencies]
anyhow = "*"

[dependencies]
anyhow = "=1.0.80"
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "anyhow".to_string(),
                semver::Version::parse("1.0.80").unwrap()
            )]
        );
    }

    #[test]
    fn read_crates_dev_dependencies() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[dev-dependencies]
anyhow = "=1.0.80"
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "anyhow".to_string(),
                semver::Version::parse("1.0.80").unwrap()
            )]
        );
    }

    #[test]
    fn read_crates_with_dev_dependencies() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[dependencies]
anyhow = "=1.0.80"

[dev-dependencies]
anyhow = "*"
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "anyhow".to_string(),
                semver::Version::parse("1.0.80").unwrap()
            )]
        );
    }

    #[test]
    fn read_crates_git() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[dependencies]
tinytable-rs = { git = "https://github.com/sukawasatoru/tinytable-rs.git", tag = "v0.3.2" }
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(actual, vec![]);
    }

    #[test]
    fn read_crates_table() {
        let source = r#"
[package]
name = "foo"
edition = 2021

[dependencies]
reqwest = { version = "=0.11.24", features = ["blocking", "json", "brotli", "gzip", "deflate"] }
"#;
        let actual = read_crates(source.as_bytes()).unwrap();
        assert_eq!(
            actual,
            vec![(
                "reqwest".to_string(),
                semver::Version::parse("0.11.24").unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn cache_load_ok() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo.etag")).unwrap())
                .write_all(b"etag value")
                .unwrap();
        }

        {
            BufWriter::new(File::create(cache_dir.join("foo")).unwrap())
                .write_all(b"value")
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let (etag, value) = cache.load(crate_name).await.unwrap();
        assert_eq!(etag.0, "etag value");
        assert_eq!(value, "value");
    }

    #[tokio::test]
    async fn cache_load_etag_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo")).unwrap())
                .write_all(b"value")
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_load_etag_open() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        std::fs::create_dir(cache_dir.join("foo.etag")).unwrap();

        {
            BufWriter::new(File::create(cache_dir.join("foo")).unwrap())
                .write_all(b"value")
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_load_etag_binary() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo.etag")).unwrap())
                .write_all(&[255])
                .unwrap();
        }

        {
            BufWriter::new(File::create(cache_dir.join("foo")).unwrap())
                .write_all(b"value")
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_load_value_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo.etag")).unwrap())
                .write_all(b"etag value")
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_load_value_open() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo.etag")).unwrap())
                .write_all(b"etag value")
                .unwrap();
        }

        std::fs::create_dir(cache_dir.join("foo")).unwrap();

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_load_value_binary() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        {
            BufWriter::new(File::create(cache_dir.join("foo.etag")).unwrap())
                .write_all(b"etag value")
                .unwrap();
        }

        {
            BufWriter::new(File::create(cache_dir.join("foo")).unwrap())
                .write_all(&[255])
                .unwrap();
        }

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        let actual = cache.load(crate_name).await;
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn cache_save_ok_new() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        cache
            .save(crate_name, &ETag("etag value".into()), "value")
            .await
            .unwrap();

        let mut actual = String::new();

        let mut reader = BufReader::new(File::open(cache_dir.join("foo.etag")).unwrap());
        reader.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, "etag value");

        actual.clear();
        let mut reader = BufReader::new(File::open(cache_dir.join("foo")).unwrap());
        reader.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, "value");
    }

    #[tokio::test]
    async fn cache_save_ok_overrite() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        cache
            .save(crate_name, &ETag("etag value 1".into()), "value 1")
            .await
            .unwrap();

        cache
            .save(crate_name, &ETag("etag value 2".into()), "value 2")
            .await
            .unwrap();

        let mut actual = String::new();

        let mut reader = BufReader::new(File::open(cache_dir.join("foo.etag")).unwrap());
        reader.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, "etag value 2");

        actual.clear();
        let mut reader = BufReader::new(File::open(cache_dir.join("foo")).unwrap());
        reader.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, "value 2");
    }

    #[tokio::test]
    async fn cache_save_etag() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        std::fs::create_dir(cache_dir.join("foo.etag")).unwrap();

        let actual = cache
            .save(crate_name, &ETag("etag value".into()), "value")
            .await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn cache_save_value() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path();

        let crate_name = "foo";

        let cache = CratesCacheFile {
            dir: cache_dir.to_owned(),
        };

        std::fs::create_dir(cache_dir.join("foo")).unwrap();

        let actual = cache
            .save(crate_name, &ETag("etag value".into()), "value")
            .await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_version_xz() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"xz","vers":"0.0.1","deps":[],"cksum":"","features":{},"yanked":false}
{"name":"xz","vers":"0.1.0","deps":[],"cksum":"","features":{},"yanked":false}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/2/xz",
            axum::routing::get(move || async {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Tue, 11 Apr 2023 17:09:33 GMT",
                        ),
                        (
                            axum::http::header::ETAG,
                            r#""baab9ec0fc5217fa7b52db8a449e504c""#,
                        ),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo.fetch_latest_version("xz", false, false).await.unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("xz").await.unwrap();
        assert_eq!(
            actual,
            (
                ETag(r#""baab9ec0fc5217fa7b52db8a449e504c""#.into()),
                source.to_owned()
            )
        )
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_version_xz2() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"xz2","vers":"0.1.0","deps":[],"cksum":"","features":{},"yanked":false}
{"name":"xz2","vers":"0.1.7","deps":[],"cksum":"","features":{},"yanked":false}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/3/x/xz2",
            axum::routing::get(move || async {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Tue, 11 Apr 2023 17:16:03 GMT",
                        ),
                        (
                            axum::http::header::ETAG,
                            r#""617c77910fbb4ae29a41a00f56c5ee21""#,
                        ),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo
            .fetch_latest_version("xz2", false, false)
            .await
            .unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 1,
                patch: 7,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("xz2").await.unwrap();
        assert_eq!(
            actual,
            (
                ETag(r#""617c77910fbb4ae29a41a00f56c5ee21""#.into()),
                source.to_owned()
            )
        )
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_version_infer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"infer","vers":"0.1.0","deps":[],"cksum":"","features":{},"yanked":false}
{"name":"infer","vers":"0.15.0","deps":[],"cksum":"","features":{},"yanked":false}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/in/fe/infer",
            axum::routing::get(move || async {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Wed, 05 Jul 2023 00:38:22 GMT",
                        ),
                        (
                            axum::http::header::ETAG,
                            r#""42ba294804d05580ebc24a6cc38b424a""#,
                        ),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo
            .fetch_latest_version("infer", false, false)
            .await
            .unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 15,
                patch: 0,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("infer").await.unwrap();
        assert_eq!(
            actual,
            (
                ETag(r#""42ba294804d05580ebc24a6cc38b424a""#.into()),
                source.to_owned()
            )
        )
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_version_yanked() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"foobar","vers":"0.1.0","deps":[],"cksum":"1234","features":{},"yanked":false}
{"name":"foobar","vers":"0.2.0","deps":[],"cksum":"1234","features":{},"yanked":true}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/fo/ob/foobar",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Wed, 05 Jul 2023 00:38:22 GMT",
                        ),
                        (axum::http::header::ETAG, r#""123abc""#),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo
            .fetch_latest_version("foobar", false, false)
            .await
            .unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("foobar").await.unwrap();
        assert_eq!(actual, (ETag(r#""123abc""#.into()), source.to_owned()))
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_version_yanked_all() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"foobar","vers":"0.1.0","deps":[],"cksum":"1234","features":{},"yanked":true}
{"name":"foobar","vers":"0.2.0","deps":[],"cksum":"1234","features":{},"yanked":true}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/fo/ob/foobar",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Wed, 05 Jul 2023 00:38:22 GMT",
                        ),
                        (axum::http::header::ETAG, r#""123abc""#),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo.fetch_latest_version("foobar", false, false).await;
        assert!(actual.is_err());

        let actual = repo.cache.load("foobar").await.unwrap();
        assert_eq!(actual, (ETag(r#""123abc""#.into()), source.to_owned()))
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_etag_old() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"foobar","vers":"0.1.0","deps":[],"cksum":"1234","features":{},"yanked":false}
{"name":"foobar","vers":"0.2.0","deps":[],"cksum":"1234","features":{},"yanked":false}
                "#
        .trim()
        .to_owned();

        let res_data = source.clone();

        let router = axum::Router::new().route(
            "/fo/ob/foobar",
            axum::routing::get(|| async move {
                (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"), // remove `;charset=utf-8`
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Wed, 05 Jul 2023 00:38:22 GMT",
                        ),
                        (axum::http::header::ETAG, r#""123abc""#),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                        (axum::http::header::ACCEPT_RANGES, "bytes"),
                    ],
                    res_data,
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        cache.save("foobar", &ETag("old".into()), "aaa").await.ok();

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo
            .fetch_latest_version("foobar", false, false)
            .await
            .unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 2,
                patch: 0,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("foobar").await.unwrap();
        assert_eq!(actual, (ETag(r#""123abc""#.into()), source.to_owned()))
    }

    #[tokio::test]
    async fn crates_io_client_fetch_latest_etag_same() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let source = r#"
{"name":"foobar","vers":"0.1.0","deps":[],"cksum":"1234","features":{},"yanked":false}
{"name":"foobar","vers":"0.2.0","deps":[],"cksum":"1234","features":{},"yanked":false}
                "#
        .trim()
        .to_owned();

        let router = axum::Router::new().route(
            "/fo/ob/foobar",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                let if_none_match = headers
                    .get(axum::http::header::IF_NONE_MATCH)
                    .unwrap()
                    .to_str()
                    .unwrap();
                assert_eq!(if_none_match, r#""123abc""#);

                (
                    axum::http::StatusCode::NOT_MODIFIED,
                    [
                        (
                            axum::http::header::LAST_MODIFIED,
                            "Wed, 05 Jul 2023 00:38:22 GMT",
                        ),
                        (axum::http::header::ETAG, r#""123abc""#),
                        (axum::http::header::CACHE_CONTROL, "public,max-age=600"),
                    ],
                )
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let cache = CratesCacheFile {
            dir: tmp_dir.path().to_owned(),
        };

        cache
            .save("foobar", &ETag(r#""123abc""#.into()), &source)
            .await
            .ok();

        let repo = CratesIOClient::create_with_base_url(reqwest::Client::new(), cache, base_url);

        let actual = repo
            .fetch_latest_version("foobar", false, false)
            .await
            .unwrap();
        assert_eq!(
            actual,
            semver::Version {
                major: 0,
                minor: 2,
                patch: 0,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }
        );

        let actual = repo.cache.load("foobar").await.unwrap();
        assert_eq!(actual, (ETag(r#""123abc""#.into()), source.to_owned()))
    }

    #[allow(unused)]
    fn enable_log() {
        tracing_subscriber::fmt::fmt()
            .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
            .init();
    }
}

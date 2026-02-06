/*
 * Copyright 2022, 2023, 2024, 2025 sukawasatoru
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

use chrono::Utc;
use clap::{CommandFactory, Parser, ValueHint};
use futures::StreamExt;
use regex::Regex;
use reqwest::StatusCode;
use rusqlite::{Connection, Transaction, named_params};
use rust_myscript::prelude::*;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Formatter;
use std::fs::{File, create_dir_all};
use std::future::Future;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;
use tinytable_rs::Attribute::{NOT_NULL, PRIMARY_KEY};
use tinytable_rs::Type::{INTEGER, TEXT};
use tinytable_rs::{Column, Table, column};
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
    #[arg(value_hint = ValueHint::FilePath, required_unless_present = "completion")]
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
    if cache_dir_sparse.exists() {
        info!("remove file caches");
        std::fs::remove_dir_all(&cache_dir_sparse)?;
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
        CratesCacheDb::create(Connection::open(cache_dir.join("cache.db"))?)?,
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

#[derive(Clone, Eq, PartialEq, Debug)]
struct ETag(String);

trait CratesCache: Send {
    fn load(&self, name: &str) -> impl Future<Output = Option<(ETag, i64, String)>> + Send;

    fn save(
        &self,
        name: &str,
        etag: &ETag,
        age: i64,
        value: &str,
    ) -> impl Future<Output = Fallible<()>> + Send;
}

struct CacheTable {
    crate_name: Arc<Column>,
    etag: Arc<Column>,
    age: Arc<Column>,
    value: Arc<Column>,
    columns: Vec<Arc<Column>>,
}

impl Default for CacheTable {
    fn default() -> Self {
        let crate_name = column("crate_name", TEXT, [PRIMARY_KEY, NOT_NULL]);
        let etag = column("etag", TEXT, [NOT_NULL]);
        let age = column("age", INTEGER, [NOT_NULL]);
        let value = column("value", TEXT, [NOT_NULL]);
        Self {
            crate_name: crate_name.clone(),
            etag: etag.clone(),
            age: age.clone(),
            value: value.clone(),
            columns: vec![crate_name, etag, age, value],
        }
    }
}

impl Table for CacheTable {
    fn name(&self) -> &str {
        "cache"
    }

    fn columns(&self) -> &[Arc<Column>] {
        &self.columns
    }
}

enum CratesCacheDbCommand {
    Load {
        crate_name: String,
        result_tx: tokio::sync::oneshot::Sender<Option<(ETag, i64, String)>>,
    },
    Save {
        crate_name: String,
        etag: ETag,
        age: i64,
        value: String,
        result_tx: tokio::sync::oneshot::Sender<()>,
    },
}

struct CratesCacheDb {
    tx: Option<std::sync::mpsc::Sender<CratesCacheDbCommand>>,
    query_thread_handle: Option<JoinHandle<()>>,
}

impl CratesCacheDb {
    #[tracing::instrument(skip_all)]
    fn create(conn: Connection) -> Fallible<Self> {
        let table = CacheTable::default();

        conn.execute_batch(
            r#"
pragma journal_mode = wal;
pragma foreign_keys = on;
"#
            .trim(),
        )?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        let db_version = conn.query_row("pragma user_version", [], |row| row.get::<_, i32>(0))?;
        match db_version {
            0 => {
                let sqls = [table.create_sql()];
                conn.execute_batch(&sqls.join(";"))?;

                conn.execute("pragma user_version = 1", ())?;
            }
            1 => (),
            _ => bail!("unsupported db version: {db_version}"),
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let query_thread_handle = Self::run_query_thread(conn, table, rx);

        Ok(Self {
            tx: Some(tx),
            query_thread_handle: Some(query_thread_handle),
        })
    }

    #[tracing::instrument(skip_all)]
    fn run_query_thread(
        mut conn: Connection,
        table: CacheTable,
        request_rx: std::sync::mpsc::Receiver<CratesCacheDbCommand>,
    ) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("query-thread".into())
            .spawn(move || {
                let span = info_span!("query-thread");
                let _enter = span.enter();
                loop {
                    let mut tx = match conn.transaction() {
                        Ok(data) => data,
                        Err(e) => {
                            error!(?e, "conn.tx");
                            return;
                        }
                    };

                    match request_rx.recv() {
                        Ok(command) => {
                            Self::command_handler(&mut tx, &table, command);
                        }
                        Err(_) => {
                            debug!("stop query thread");
                            if let Err(e) = tx.commit() {
                                error!(?e, "tx.commit");
                            }
                            return;
                        }
                    }

                    let mut is_batch = false;
                    loop {
                        match request_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                            Ok(command) => {
                                debug!("batch");
                                is_batch = true;
                                Self::command_handler(&mut tx, &table, command);
                            }
                            Err(RecvTimeoutError::Timeout) if is_batch => {
                                debug!("batch end");
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => {
                                debug!("stop query thread");
                                if let Err(e) = tx.commit() {
                                    error!(?e, "tx.commit");
                                }
                                return;
                            }
                        }
                    }

                    if let Err(e) = tx.commit() {
                        error!(?e, "tx.commit");
                        return;
                    }
                }
            })
            .expect("failed to spawn thread")
    }

    #[tracing::instrument(skip_all)]
    fn command_handler(tx: &mut Transaction, table: &CacheTable, command: CratesCacheDbCommand) {
        match command {
            CratesCacheDbCommand::Load {
                crate_name,
                result_tx,
            } => {
                debug!(%crate_name, "load");
                Self::select_crate(tx, table, crate_name, result_tx)
            }
            CratesCacheDbCommand::Save {
                crate_name,
                etag,
                age,
                value,
                result_tx,
            } => {
                debug!(%crate_name, "save");
                Self::upsert_crate(tx, table, crate_name, etag, age, value, result_tx)
            }
        };
    }

    #[tracing::instrument(skip(tx, table, result_tx))]
    fn select_crate(
        tx: &mut Transaction,
        table: &CacheTable,
        crate_name: String,
        result_tx: tokio::sync::oneshot::Sender<Option<(ETag, i64, String)>>,
    ) {
        let sql = format!(
            "select {etag}, {age}, {value} from {table} where {crate_name} = :crate_name",
            etag = table.etag.name(),
            age = table.age.name(),
            value = table.value.name(),
            table = table.name(),
            crate_name = table.crate_name.name(),
        );

        let mut stmt = match tx.prepare_cached(&sql) {
            Ok(data) => data,
            Err(e) => {
                warn!(?e, "tx.stmt");
                if result_tx.send(None).is_err() {
                    warn!("send tx.stmt result");
                }
                return;
            }
        };

        let ret_query = stmt.query(named_params! {
            ":crate_name": crate_name,
        });

        let mut rows = match ret_query {
            Ok(data) => data,
            Err(e) => {
                warn!(?e, "stmt.query");
                if result_tx.send(None).is_err() {
                    warn!("send stmt.query result");
                }
                return;
            }
        };

        match rows.next() {
            Ok(Some(row)) => {
                debug!("found");
                let etag = match row.get::<_, String>(table.etag.name()) {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(?e, "row.get(etag)");
                        if result_tx.send(None).is_err() {
                            warn!("send row.get(etag) result");
                        }
                        return;
                    }
                };
                let age = match row.get::<_, i64>(table.age.name()) {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(?e, "row.get(age)");
                        if result_tx.send(None).is_err() {
                            warn!("send row.get(age) result");
                        }
                        return;
                    }
                };
                let value = match row.get::<_, String>(table.value.name()) {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(?e, "row.get(value)");
                        if result_tx.send(None).is_err() {
                            warn!("send row.get(value) result");
                        }
                        return;
                    }
                };
                if result_tx.send(Some((ETag(etag), age, value))).is_err() {
                    warn!("send succeeded result");
                }
            }
            Ok(None) => {
                debug!("not found");
                if result_tx.send(None).is_err() {
                    warn!("send none result");
                }
            }
            Err(e) => {
                warn!(?e, "rows.next");
                if result_tx.send(None).is_err() {
                    warn!("send rows.next result");
                }
            }
        }
    }

    #[tracing::instrument(skip(tx, table, result_tx))]
    fn upsert_crate(
        tx: &mut Transaction,
        table: &CacheTable,
        crate_name: String,
        etag: ETag,
        age: i64,
        value: String,
        result_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let sql = format!(
            "insert or replace into {table} ({crate_name}, {etag}, {age}, {value}) values(:crate_name, :etag, :age, :value)",
            table = table.name(),
            crate_name = table.crate_name.name(),
            etag = table.etag.name(),
            age = table.age.name(),
            value = table.value.name(),
        );

        let mut stmt = match tx.prepare_cached(&sql) {
            Ok(data) => data,
            Err(e) => {
                warn!(?e, "tx.stmt");
                if result_tx.send(()).is_err() {
                    warn!("send tx.stmt result");
                }
                return;
            }
        };

        let ret_query = stmt.execute(named_params! {
            ":crate_name": crate_name,
            ":etag": etag.0,
            ":age": age,
            ":value": value,
        });
        drop(stmt);

        match ret_query {
            Ok(1) => {}
            Ok(num) => warn!(%num, "unexpected affect num"),
            Err(e) => warn!(?e, "stmt.query"),
        };

        if result_tx.send(()).is_err() {
            warn!("send result");
        }
    }
}

impl Drop for CratesCacheDb {
    fn drop(&mut self) {
        let _ = self.tx.take().expect("CratesCacheDb.tx should be Some()");

        self.query_thread_handle
            .take()
            .expect("CratesCacheDb.handle should be Some()")
            .join()
            .unwrap();
    }
}

impl CratesCache for CratesCacheDb {
    #[tracing::instrument(skip(self))]
    async fn load(&self, name: &str) -> Option<(ETag, i64, String)> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        match self
            .tx
            .as_ref()
            .expect("CratesCacheDb.tx should be Some()")
            .send(CratesCacheDbCommand::Load {
                crate_name: name.to_owned(),
                result_tx: tx,
            }) {
            Ok(_) => {}
            Err(_) => {
                warn!("failed to send request");
                return None;
            }
        }

        match rx.await {
            Ok(data) => data,
            Err(_) => {
                warn!("thread closed");
                return None;
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn save(&self, name: &str, etag: &ETag, age: i64, value: &str) -> Fallible<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .as_ref()
            .expect("CratesCacheDb.tx should be Some()")
            .send(CratesCacheDbCommand::Save {
                crate_name: name.to_owned(),
                etag: etag.clone(),
                age,
                value: value.to_owned(),
                result_tx: tx,
            })
            .context("failed to send request")?;

        match rx.await {
            Ok(_) => Ok(()),
            Err(_) => bail!("thread closed"),
        }
    }
}

fn init_reg_max_age() -> Regex {
    Regex::new(r#"max-age=(\d+)"#).expect("max-age")
}

trait TimestampProvider {
    fn timestamp(&self) -> i64;
}

struct DefaultTimestampProvider;

impl TimestampProvider for DefaultTimestampProvider {
    fn timestamp(&self) -> i64 {
        Utc::now().timestamp()
    }
}

struct CratesIOClient<Cache: CratesCache, TP: TimestampProvider> {
    client: reqwest::Client,
    base_url: Url,
    cache: Cache,
    timestamp_provider: TP,
}

impl<Cache: CratesCache> CratesIOClient<Cache, DefaultTimestampProvider> {
    fn create(client: reqwest::Client, cache: Cache) -> Self {
        Self::create_impl(
            client,
            cache,
            Url::parse("https://index.crates.io").expect("base_url"),
            DefaultTimestampProvider,
        )
    }
}

impl<Cache: CratesCache, TP: TimestampProvider> CratesIOClient<Cache, TP> {
    fn create_impl(
        client: reqwest::Client,
        cache: Cache,
        base_url: Url,
        timestamp_provider: TP,
    ) -> Self {
        Self {
            client,
            base_url,
            cache,
            timestamp_provider,
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

        let compute_age = |res: &reqwest::Response| {
            let age = res
                .headers()
                .get(reqwest::header::AGE)
                .and_then(|data| data.to_str().ok())
                .and_then(|data| data.parse::<u32>().ok())
                .unwrap_or(0);

            static REG_MAX_AGE: LazyLock<Regex> = LazyLock::new(init_reg_max_age);
            res.headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|data| data.to_str().ok())
                .and_then(|data| REG_MAX_AGE.captures(data))
                .and_then(|data| data.get(1))
                .and_then(|data| data.as_str().parse::<u32>().ok())
                .map(|max_age| self.timestamp_provider.timestamp() + i64::from(max_age - age))
                .unwrap_or(0)
        };

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

            let age = compute_age(&res);

            debug!("retrieve body");
            let text = res.text().await?;

            if let Some(etag) = etag
                && let Err(e) = self.cache.save(crate_name, &ETag(etag), age, &text).await
            {
                warn!(?e, crate_name, "failed to store cache");
            }

            Result::<String, anyhow::Error>::Ok(text)
        };

        let text = match self.cache.load(crate_name).await {
            Some((etag, age, text)) if !force => {
                if age < self.timestamp_provider.timestamp() {
                    debug!(etag = %etag.0, %age, "request w/ etag");
                    let res = request_get(builder.header(reqwest::header::IF_NONE_MATCH, &etag.0))
                        .await?;

                    if res.status() == StatusCode::NOT_MODIFIED {
                        debug!("use cache");
                        let age = compute_age(&res);
                        if let Err(e) = self.cache.save(crate_name, &etag, age, &text).await {
                            warn!(?e, "failed to update age");
                        }
                        text
                    } else {
                        retrieve_and_store_text(res).await?
                    }
                } else {
                    debug!(etag = %etag.0, %age, "request skip (use cache)");
                    text
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
    impl Visitor<'_> for VersionString {
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

#[cfg(test)]
mod tests {
    use super::*;
    use semver::{BuildMetadata, Prerelease};

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
    async fn cache_db_save_ok_new() {
        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        cache
            .save("foo", &ETag("etag value".into()), 1, "value")
            .await
            .unwrap();

        let (etag, age, value) = cache.load("foo").await.unwrap();

        assert_eq!(etag, ETag("etag value".into()));
        assert_eq!(age, 1);
        assert_eq!(value, "value");
    }

    #[tokio::test]
    async fn cache_db_save_ok_overwrite() {
        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        cache
            .save("foo", &ETag("etag value 1".into()), 1, "value 1")
            .await
            .unwrap();

        cache
            .save("foo", &ETag("etag value 2".into()), 2, "value 2")
            .await
            .unwrap();

        let (etag, age, value) = cache.load("foo").await.unwrap();

        assert_eq!(etag, ETag("etag value 2".into()));
        assert_eq!(age, 2);
        assert_eq!(value, "value 2");
    }

    struct TestTimestampProvider;
    impl TimestampProvider for TestTimestampProvider {
        fn timestamp(&self) -> i64 {
            0
        }
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
                600,
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
                600,
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
                600,
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
        assert_eq!(actual, (ETag(r#""123abc""#.into()), 600, source.to_owned()))
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

        let actual = repo.fetch_latest_version("foobar", false, false).await;
        assert!(actual.is_err());

        let actual = repo.cache.load("foobar").await.unwrap();
        assert_eq!(actual, (ETag(r#""123abc""#.into()), 600, source.to_owned()))
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        cache
            .save("foobar", &ETag("old".into()), -1, "aaa")
            .await
            .ok();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
        assert_eq!(actual, (ETag(r#""123abc""#.into()), 600, source.to_owned()))
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

        let cache = CratesCacheDb::create(Connection::open_in_memory().unwrap()).unwrap();

        cache
            .save("foobar", &ETag(r#""123abc""#.into()), -1, &source)
            .await
            .ok();

        let repo = CratesIOClient::create_impl(
            reqwest::Client::new(),
            cache,
            base_url,
            TestTimestampProvider,
        );

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
        assert_eq!(actual, (ETag(r#""123abc""#.into()), 600, source.to_owned()))
    }

    #[allow(unused)]
    fn enable_log() {
        tracing_subscriber::fmt::fmt()
            .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
            .init();
    }
}

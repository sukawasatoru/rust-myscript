use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::{named_params, CachedStatement, Connection, Rows, Transaction};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::marker::PhantomPinned;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::Arc;
use tinytable_rs::Attribute::{NOT_NULL, PRIMARY_KEY};
use tinytable_rs::Type::{INTEGER, TEXT};
use tinytable_rs::{column, Column, Table};

#[derive(Debug, Parser)]
struct Opt {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(name = "trim")]
    Trim {
        /// Backup a FILE to specified path
        #[arg(short, long = "backup")]
        backup_path: Option<PathBuf>,

        /// history file
        #[arg(name = "FILE")]
        history_path: PathBuf,
    },
    #[command(name = "show")]
    Show {
        /// prints the first NUM lines
        #[arg(name = "NUM", short, long = "lines")]
        num: Option<u32>,
    },
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
struct Entry {
    command: String,
    count: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct Statistics {
    entries: Vec<Entry>,
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt: Opt = Opt::parse();
    debug!(?opt);

    match opt.cmd {
        Command::Trim {
            backup_path,
            history_path,
        } => trim_from_path(history_path, backup_path),
        Command::Show { num } => show(num),
    }
}

#[tracing::instrument]
fn trim_from_path(history_path: PathBuf, backup_path: Option<PathBuf>) -> Fallible<()> {
    let project_dirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").context("ProjectDirs")?;
    let statistics_toml_path = project_dirs.data_dir().join("statistics.toml");

    statistics_toml_path
        .parent()
        .context("statistics parent")
        .and_then(|parent| {
            if !parent.exists() {
                create_dir_all(parent).context("create statistics directory")
            } else {
                Ok(())
            }
        })?;

    let statistics_db_path = project_dirs.data_dir().join("statistics.db");

    if statistics_toml_path.exists() {
        if !statistics_db_path.exists() {
            let reader = BufReader::new(File::open(&statistics_toml_path)?);
            let mut db = Db::create_with_path(&statistics_db_path)?;
            if let Err(e) = import_from_toml(reader, &mut db) {
                drop(db);
                std::fs::remove_file(&statistics_db_path)?;
                return Err(e);
            }
        }
        std::fs::remove_file(&statistics_toml_path)?;
    }

    let mut db = Db::create_with_path(&statistics_db_path)?;

    let backup_dest = match backup_path {
        Some(dest) => Some(BufWriter::new(File::create(dest)?)),
        None => None,
    };

    trim(
        (BufReader::new(File::open(&history_path)?), || {
            Ok(BufWriter::new(File::create(&history_path)?))
        }),
        backup_dest,
        &mut db,
    )
}

fn import_from_toml<TomlSrc: Read>(toml_src: TomlSrc, db: &mut Db) -> Fallible<()> {
    let statistics = load_statistics(toml_src)?;

    let mut tx = db.tx()?;
    tx.insert_entries(&statistics.entries)?;
    tx.commit()?;

    Ok(())
}

fn trim<HistorySrc, HistoryDest, HistoryDestProvider, BackupDest>(
    history: (HistorySrc, HistoryDestProvider),
    mut backup_dest: Option<BackupDest>,
    db: &mut Db,
) -> Fallible<()>
where
    HistorySrc: BufRead,
    HistoryDest: Write,
    HistoryDestProvider: FnOnce() -> Fallible<HistoryDest>,
    BackupDest: Write,
{
    let (mut history_src, history_dest_provider) = history;

    let mut line = String::new();
    let mut trimmed = Vec::new();
    let mut trim_count = 0;
    let mut tx = db.tx()?;
    loop {
        match history_src.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if let Some(ref mut backup_dest) = backup_dest {
                    write!(backup_dest, "{line}")?;
                }

                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                debug!(result = ?line);
                let trimmed_line = line.trim();
                if let Some(index) = trimmed.iter().position(|entity| trimmed_line == entity) {
                    debug!(index, "contains");
                    trimmed.remove(index);
                    trim_count += 1;
                    tx.add_or_increment(trimmed_line)?;
                }
                trimmed.push(trimmed_line.to_owned());
                line.clear();
            }
            Err(e) => return Err(e.into()),
        }
    }
    tx.commit()?;

    if let Some(ref mut backup_dest) = backup_dest {
        backup_dest.flush()?;
    }

    info!(trim_count, trimmed_len = trimmed.len());

    let mut history_dest = history_dest_provider()?;
    for entity in trimmed.iter() {
        writeln!(&mut history_dest, "{entity}")?;
    }
    history_dest.flush()?;

    Ok(())
}

fn show(num: Option<u32>) -> Fallible<()> {
    let project_dirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").context("ProjectDirs")?;
    let statistics_toml_path = project_dirs.data_dir().join("statistics.toml");

    let statistics_db_path = project_dirs.data_dir().join("statistics.db");

    if statistics_toml_path.exists() {
        if !statistics_db_path.exists() {
            let reader = BufReader::new(File::open(&statistics_toml_path)?);
            let mut db = Db::create_with_path(&statistics_db_path)?;
            if let Err(e) = import_from_toml(reader, &mut db) {
                drop(db);
                std::fs::remove_file(&statistics_db_path)?;
                return Err(e);
            }
        }
        std::fs::remove_file(&statistics_toml_path)?;
    }

    let mut db = Db::create_with_path(&statistics_db_path)?;
    let tx = db.tx()?;

    for entry in tx.find_all(num)? {
        let entry = entry?;
        println!("{:4}: {}", entry.count, entry.command);
    }

    Ok(())
}

fn load_statistics<R: Read>(mut src: R) -> Fallible<Statistics> {
    let mut buf = String::new();
    src.read_to_string(&mut buf)?;
    toml::from_str(&buf).context("failed to parse statistics")
}

struct StatisticsTable {
    command: Arc<Column>,
    count: Arc<Column>,
    created_at: Arc<Column>,
    columns: Vec<Arc<Column>>,
    indexes: Vec<(String, Vec<Arc<Column>>)>,
}

impl Default for StatisticsTable {
    fn default() -> Self {
        let command = column("command", TEXT, [PRIMARY_KEY, NOT_NULL]);
        let count = column("count", INTEGER, [NOT_NULL]);
        let created_at = column("created_at", INTEGER, [NOT_NULL]);
        Self {
            command: command.clone(),
            count: count.clone(),
            created_at: created_at.clone(),
            columns: vec![command, count.clone(), created_at.clone()],
            indexes: vec![(
                "index_count_created_at".to_owned(),
                vec![
                    // TODO: asc/desc.
                    column(format!("{} desc", count.name()), INTEGER, []),
                    column(format!("{} desc", created_at.name()), INTEGER, []),
                ],
            )],
        }
    }
}

impl Table for StatisticsTable {
    fn name(&self) -> &str {
        "statistics"
    }

    fn columns(&self) -> &[Arc<Column>] {
        &self.columns
    }

    fn indexes(&self) -> &[(String, Vec<Arc<Column>>)] {
        &self.indexes
    }
}

struct Db {
    conn: Connection,
    table: StatisticsTable,
}

impl Db {
    fn create_with_path(db_path: &Path) -> Fallible<Self> {
        Self::create_with_conn(Connection::open(db_path)?)
    }

    fn create_with_conn(conn: Connection) -> Fallible<Self> {
        let table = StatisticsTable::default();

        let db_version = conn.query_row("pragma user_version", [], |row| row.get::<_, i32>(0))?;
        match db_version {
            0 => {
                let mut sqls = vec![table.create_sql()];
                sqls.extend(table.create_index());
                conn.execute_batch(&sqls.join(";"))?;

                conn.execute("pragma user_version = 1", ())?;
            }
            1 => (),
            _ => bail!("unsupported db version: {db_version}"),
        }

        Ok(Self { conn, table })
    }

    fn tx(&mut self) -> Fallible<DbTx> {
        Ok(DbTx {
            tx: self.conn.transaction().context("conn.tx")?,
            table: &self.table,
        })
    }
}

struct DbTx<'a> {
    tx: Transaction<'a>,
    table: &'a StatisticsTable,
}

impl DbTx<'_> {
    fn commit(self) -> Fallible<()> {
        self.tx.commit().context("tx.commit")
    }

    fn insert_entries(&mut self, entries: &[Entry]) -> Fallible<()> {
        let current_time = Utc::now();

        let sql = format!(
            "insert into {table} ({command}, {count}, {created_at}) values (:command, :count, :created_at)",
            table = self.table.name(),
            command = self.table.command.name(),
            count = self.table.count.name(),
            created_at = self.table.created_at.name(),
        );
        let mut stmt = self.tx.prepare_cached(&sql).context("tx.prepare")?;
        for entry in entries {
            stmt.insert(named_params! {
                ":command": entry.command,
                ":count": entry.count,
                ":created_at": current_time.timestamp(),
            })
            .with_context(|| format!("stmt.execute: {}", entry.command))?;
        }

        Ok(())
    }

    fn add_or_increment(&mut self, command_name: &str) -> Fallible<()> {
        let current_time = Utc::now();

        let sql = format!(
            "insert or replace into {table} ({command}, {count}, {created_at}) select {command}, {count} + 1 as {count}, {created_at} from {table} where {command} = :command union select :command as {command}, 1 as {count}, :created_at as {created_at} where not exists(select 1 from {table} where {command} = :command)",
            table = self.table.name(),
            command = self.table.command.name(),
            count = self.table.count.name(),
            created_at = self.table.created_at.name(),
        );

        let mut stmt = self.tx.prepare_cached(&sql).context("tx.prepare")?;
        stmt.insert(named_params! {
            ":command": command_name,
            ":created_at": current_time.timestamp(),
        })
        .context("stmt.insert")?;

        Ok(())
    }

    fn find_all(&self, limit: Option<u32>) -> Fallible<EntryRows<'_>> {
        let mut sql = format!(
            "select {command}, {count} from {table} order by {count} desc, {created_at} desc",
            command = self.table.command.name(),
            count = self.table.count.name(),
            created_at = self.table.created_at.name(),
            table = self.table.name(),
        );

        if let Some(limit) = limit {
            sql.push_str(&format!(" limit {limit}"));
        }

        let stmt = self.tx.prepare_cached(&sql).context("tx.prepare")?;
        let index_command = stmt
            .column_index(self.table.command.name())
            .context("stmt.index(command)")?;
        let index_count = stmt
            .column_index(self.table.count.name())
            .context("stmt.index(count)")?;

        let stmt = Box::leak(Box::new(stmt));
        let (stmt, rows) = unsafe {
            let mut stmt = NonNull::new_unchecked(stmt as *mut CachedStatement);

            let rows = match stmt.as_mut().query([]) {
                Ok(rows) => rows,
                Err(e) => {
                    let _ = Box::from_raw(stmt.as_ptr());
                    return Err(e).context("stmt.query");
                }
            };
            let rows = Box::leak(Box::new(rows));
            let rows = NonNull::new_unchecked(rows as *mut Rows);
            (stmt, rows)
        };

        Ok(EntryRows::create(stmt, rows, index_command, index_count))
    }
}

struct EntryRows<'conn> {
    inner: Pin<Box<EntryRowsInner<'conn>>>,
}

impl<'conn> EntryRows<'conn> {
    fn create(
        stmt: NonNull<CachedStatement<'conn>>,
        rows: NonNull<Rows<'conn>>,
        index_command: usize,
        index_count: usize,
    ) -> Self {
        Self {
            inner: Box::pin(EntryRowsInner {
                stmt,
                rows,
                index_command,
                index_count,
                _pin: PhantomPinned,
            }),
        }
    }
}

impl Iterator for EntryRows<'_> {
    type Item = Fallible<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        let inner = unsafe { self.inner.as_mut().get_unchecked_mut() };
        inner.next()
    }
}

struct EntryRowsInner<'conn> {
    stmt: NonNull<CachedStatement<'conn>>,
    rows: NonNull<Rows<'conn>>,
    index_command: usize,
    index_count: usize,
    _pin: PhantomPinned,
}

impl Drop for EntryRowsInner<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(self.rows.as_ptr());
            let _ = Box::from_raw(self.stmt.as_ptr());
        }
    }
}

impl Iterator for EntryRowsInner<'_> {
    type Item = Fallible<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        let rows = unsafe { self.rows.as_mut() };
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => return None,
            Err(e) => return Some(Err(e.into())),
        };

        let command = match row.get(self.index_command) {
            Ok(data) => data,
            Err(e) => return Some(Err(e.into())),
        };

        let count = match row.get(self.index_count) {
            Ok(data) => data,
            Err(e) => return Some(Err(e.into())),
        };

        Some(Ok(Entry { command, count }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }

    #[test]
    fn test_trim() {
        let history_src = r#"
pwd
pwd
ls
ls -l
cd ~/
pwd
cd ~/
"#;

        let mut actual_history = Vec::new();
        let history_writer = BufWriter::new(&mut actual_history);

        let mut actual_backup = Vec::new();
        let backup_writer = BufWriter::new(&mut actual_backup);

        let mut db = Db::create_with_conn(Connection::open_in_memory().unwrap()).unwrap();

        let mut tx = db.tx().unwrap();
        tx.insert_entries(&[
            Entry {
                command: "pwd".to_string(),
                count: 4113,
            },
            Entry {
                command: "ls -l".to_string(),
                count: 416,
            },
            Entry {
                command: "ls -la".to_string(),
                count: 317,
            },
        ])
        .unwrap();
        tx.commit().unwrap();

        trim(
            (history_src.as_bytes(), || Ok(history_writer)),
            Some(backup_writer),
            &mut db,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(actual_history).unwrap(),
            r#"
ls
ls -l
pwd
cd ~/
"#
        );

        assert_eq!(String::from_utf8(actual_backup).unwrap(), history_src);

        let tx = db.tx().unwrap();
        let mut rows = tx.find_all(None).unwrap();
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "pwd".to_string(),
                count: 4115,
            }
        );
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "ls -l".to_string(),
                count: 416,
            }
        );
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "ls -la".to_string(),
                count: 317,
            }
        );
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "cd ~/".to_string(),
                count: 1,
            }
        );
    }

    #[test]
    fn db_insert_query() {
        let mut db = Db::create_with_conn(Connection::open_in_memory().unwrap()).unwrap();
        let mut tx = db.tx().unwrap();

        tx.add_or_increment("command 3").unwrap();
        tx.add_or_increment("command 3").unwrap();
        tx.add_or_increment("command 1").unwrap();
        tx.add_or_increment("command 1").unwrap();
        tx.add_or_increment("command 1").unwrap();
        tx.add_or_increment("command 1").unwrap();
        tx.add_or_increment("command 1").unwrap();
        tx.add_or_increment("command 2").unwrap();

        tx.commit().unwrap();

        let tx = db.tx().unwrap();
        let mut rows = tx.find_all(None).unwrap();

        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "command 1".to_string(),
                count: 5,
            }
        );

        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "command 3".to_string(),
                count: 2,
            }
        );

        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "command 2".to_string(),
                count: 1,
            }
        );

        assert!(rows.next().is_none());
        drop(rows);

        let mut rows = tx.find_all(Some(1)).unwrap();
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: "command 1".to_string(),
                count: 5,
            }
        );
        assert!(rows.next().is_none());
    }

    #[test]
    fn db_insert_rollback() {
        let mut db = Db::create_with_conn(Connection::open_in_memory().unwrap()).unwrap();

        let entry1 = Entry {
            command: "command 1".to_owned(),
            count: 10,
        };
        let entry2 = Entry {
            command: "command 2".to_owned(),
            count: 5,
        };

        let mut tx = db.tx().unwrap();
        tx.insert_entries(&[
            Entry {
                command: entry1.command.clone(),
                count: entry1.count,
            },
            Entry {
                command: entry2.command.clone(),
                count: entry2.count,
            },
        ])
        .unwrap();
        tx.commit().unwrap();

        let tx = db.tx().unwrap();
        let mut rows = tx.find_all(None).unwrap();
        assert_eq!(rows.next().unwrap().unwrap(), entry1);
        assert_eq!(rows.next().unwrap().unwrap(), entry2);
        assert!(rows.next().is_none());
        drop(rows);
        drop(tx);

        let mut tx = db.tx().unwrap();
        tx.add_or_increment(&entry1.command).unwrap();
        let mut rows = tx.find_all(None).unwrap();
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Entry {
                command: entry1.command.clone(),
                count: entry1.count + 1
            }
        );
        assert_eq!(rows.next().unwrap().unwrap(), entry2);
        assert!(rows.next().is_none());
        drop(rows);
        drop(tx);

        let tx = db.tx().unwrap();
        let mut rows = tx.find_all(None).unwrap();
        assert_eq!(rows.next().unwrap().unwrap(), entry1);
        assert_eq!(rows.next().unwrap().unwrap(), entry2);
        assert!(rows.next().is_none());
    }
}

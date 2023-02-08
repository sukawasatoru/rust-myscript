/*
 * Copyright 2020, 2021, 2022, 2023 sukawasatoru
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

use clap::{Parser, Subcommand};
use digest::Digest;
use directories::ProjectDirs;
use rusqlite::params;
use rust_myscript::prelude::*;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tinytable_rs::Attribute::{NOT_NULL, PRIMARY_KEY};
use tinytable_rs::Type::TEXT;
use tinytable_rs::{column, Column, Table};
use tracing::info;

#[derive(Parser)]
struct Opt {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check password
    Check {
        #[command(subcommand)]
        cmd: CheckCommand,
    },

    /// Create database for query password hash
    Create {
        /// File path. e.g. pwned-passwords-sha1-ordered-by-hash-v6.txt
        file: PathBuf,

        /// Database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CheckCommand {
    /// Use online backend for query password hash
    Net,

    /// Use SQLite database for query password hash
    Db {
        /// Database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

struct HexFormat<'a>(&'a [u8]);

impl<'a> std::fmt::Display for HexFormat<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for entry in self.0 {
            write!(f, "{entry:02X?}")?;
        }

        Ok(())
    }
}

struct PasswordTable {
    col_hash: Arc<Column>,
    col_count: Arc<Column>,
    column: Vec<Arc<Column>>,
}

impl PasswordTable {
    fn new() -> Self {
        let col_hash = column("hash", TEXT, [PRIMARY_KEY, NOT_NULL]);
        let col_count = column("count", TEXT, [NOT_NULL]);
        Self {
            col_hash: col_hash.clone(),
            col_count: col_count.clone(),
            column: vec![col_hash, col_count],
        }
    }
}

impl Table for PasswordTable {
    fn name(&self) -> &str {
        "password"
    }

    fn columns(&self) -> &[Arc<Column>] {
        &self.column
    }
}

// https://haveibeenpwned.com/passwords
fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt: Opt = Opt::parse();

    info!("Hello");

    let project_dirs =
        ProjectDirs::from("jp", "tinyport", "pwnedpassword").context("project_dirs")?;

    match opt.cmd {
        Command::Check { cmd } => {
            #[cfg(target_os = "windows")]
            {
                eprintln!("Please input password and Ctrl+Z");
            }

            #[cfg(not(target_os = "windows"))]
            {
                eprintln!("Please input password and Ctrl+D");
            }

            let mut input_lines = String::new();
            std::io::stdin().read_to_string(&mut input_lines).ok();

            let db_path = if let CheckCommand::Db { ref db } = cmd {
                db.as_ref()
                    .map(|data| data.to_path_buf())
                    .unwrap_or_else(|| default_db_path(&project_dirs))
            } else {
                PathBuf::new()
            };

            let mut first_pwned = false;
            for entry in input_lines.split('\n') {
                let plain_password = entry.trim();
                if plain_password.is_empty() {
                    continue;
                }

                let pwned = match cmd {
                    CheckCommand::Db { .. } => {
                        check_data_source_db(&db_path, plain_password)?.is_some()
                    }
                    CheckCommand::Net => check_data_source_net(plain_password)?.is_some(),
                };

                if pwned {
                    if !first_pwned {
                        first_pwned = true;
                        println!();
                    }
                    println!("pwned: {plain_password}");
                }
            }
        }
        Command::Create { file, db } => {
            create_db(&db.unwrap_or_else(|| default_db_path(&project_dirs)), &file)?
        }
    }

    info!("Bye");

    Ok(())
}

fn create_db(db_path: &Path, file_path: &Path) -> Fallible<()> {
    // curl --head https://downloads.pwnedpasswords.com/passwords/pwned-passwords-sha1-ordered-by-hash-v6.7z
    // check etag.
    // 7z x 7z x pwned-passwords-sha1-ordered-by-hash-v6.7z
    // -> pwned-passwords-sha1-ordered-by-hash-v6.txt
    // -> 10GB (7z) -> 25GB (txt)

    let mut reader = std::io::BufReader::new(std::fs::File::open(file_path)?);
    let mut buf = String::new();

    std::fs::create_dir_all(db_path.parent().context("db_path.parent")?)
        .context("create_dir_all(db_path)")?;
    let mut conn = rusqlite::Connection::open(db_path)?;
    let password_table = PasswordTable::new();
    conn.execute(&password_table.create_sql(), [])?;
    let transaction = conn.transaction()?;
    let mut insert_statement = transaction.prepare(&format!(
        "INSERT INTO {} ({}, {}) VALUES(?, ?)",
        password_table.name(),
        password_table.col_hash.name(),
        password_table.col_count.name()
    ))?;

    loop {
        buf.clear();
        if let 0 = reader.read_line(&mut buf)? {
            break;
        }

        let mut segments = buf.trim().split(':');
        let hash = segments.next().context("hash")?;
        let count = segments.next().context("count")?;
        insert_statement.insert(params![hash, count])?;
    }

    drop(insert_statement);
    transaction.commit()?;

    Ok(())
}

fn check_data_source_db(db_path: &Path, plain_password: &str) -> anyhow::Result<Option<()>> {
    let password_hash =
        HexFormat(sha1::Sha1::digest(plain_password.as_bytes()).as_slice()).to_string();
    let conn = rusqlite::Connection::open(db_path)?;
    let password_table = PasswordTable::new();
    let ret = conn
        .prepare_cached(&format!(
            "SELECT count(*) FROM {} WHERE hash = ?",
            password_table.name()
        ))?
        .query_map(params![password_hash], |row| Ok(0 < row.get::<_, i32>(0)?))?
        .into_iter()
        .next()
        .context("SELECT count(*)")??;

    if ret {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

fn check_data_source_net(plain_password: &str) -> anyhow::Result<Option<()>> {
    // https://haveibeenpwned.com/API/v3
    let password_hash =
        HexFormat(sha1::Sha1::digest(plain_password.as_bytes()).as_slice()).to_string();

    let response = reqwest::blocking::get(format!(
        "https://api.pwnedpasswords.com/range/{}",
        &password_hash[0..5]
    ))?
    .text()?;

    for line in response.split('\n') {
        let hash_segment = line.split(':').next().context("hash segment")?;
        if &password_hash[5..] == hash_segment {
            return Ok(Some(()));
        }
    }

    Ok(None)
}

fn default_db_path(project_dirs: &ProjectDirs) -> PathBuf {
    project_dirs.data_dir().join("pwned-password-sha1.sqlite")
}

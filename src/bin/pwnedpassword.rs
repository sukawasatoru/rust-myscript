use digest::Digest;
use log::info;
use rusqlite::{params, NO_PARAMS};
use rust_myscript::prelude::*;
use std::io::prelude::*;
use std::sync::Arc;
use structopt::StructOpt;
use tinytable_rs::Attribute::{NOT_NULL, PRIMARY_KEY};
use tinytable_rs::Type::TEXT;
use tinytable_rs::{column, Column, Table};

#[derive(StructOpt)]
struct Opt {
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt)]
enum Command {
    /// Check password
    Check {
        #[structopt(subcommand)]
        cmd: CheckCommand,
    },

    /// Create database for query password hash
    Create,
}

#[derive(StructOpt)]
enum CheckCommand {
    /// Use online backend for query password hash
    Net,

    /// Use SQLite database for query password hash
    Db,
}

struct HexFormat<'a>(&'a [u8]);

impl<'a> std::fmt::Display for HexFormat<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for entry in self.0 {
            write!(f, "{:02X?}", entry)?;
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

const TEXT_NAME: &str = "pwned-passwords-sha1-ordered-by-hash-v6.txt";
const DB_NAME: &str = "pwned-password-sha1.sqlite";

// https://haveibeenpwned.com/passwords
fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    let opt: Opt = Opt::from_args();

    info!("Hello");

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

            let mut first_pwned = false;
            for entry in input_lines.split('\n') {
                let plain_password = entry.trim();
                if plain_password.is_empty() {
                    continue;
                }

                let pwned = match cmd {
                    CheckCommand::Db => check_data_source_db(plain_password)?.is_some(),
                    CheckCommand::Net => check_data_source_net(plain_password)?.is_some(),
                };

                if pwned {
                    if !first_pwned {
                        first_pwned = true;
                        println!();
                    }
                    println!("pwned: {}", plain_password);
                }
            }
        }
        Command::Create => create_db()?,
    }

    info!("Bye");

    Ok(())
}

fn create_db() -> anyhow::Result<()> {
    // curl --head https://downloads.pwnedpasswords.com/passwords/pwned-passwords-sha1-ordered-by-hash-v6.7z
    // check etag.
    // 7z x 7z x pwned-passwords-sha1-ordered-by-hash-v6.7z
    // -> pwned-passwords-sha1-ordered-by-hash-v6.txt
    // -> 10GB (7z) -> 25GB (txt)

    let mut reader = std::io::BufReader::new(std::fs::File::open(TEXT_NAME)?);
    let mut buf = String::new();

    let mut conn = rusqlite::Connection::open(DB_NAME)?;
    let password_table = PasswordTable::new();
    conn.execute(&password_table.create_sql(), NO_PARAMS)?;
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

fn check_data_source_db(plain_password: &str) -> anyhow::Result<Option<()>> {
    let password_hash =
        HexFormat(sha1::Sha1::digest(plain_password.as_bytes()).as_slice()).to_string();
    let conn = rusqlite::Connection::open(DB_NAME)?;
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

    let response = reqwest::blocking::get(&format!(
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

use clap::{ArgGroup, Parser};
use rusqlite::Connection;
use rust_myscript::model::SQLiteUserVersion;
use rust_myscript::prelude::*;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(group = ArgGroup::new("source").required(true))]
struct Opt {
    #[arg(short, long, group = "source")]
    database_path: Option<PathBuf>,

    #[arg(name = "USER-VERSION", group = "source")]
    version_string: Option<String>,
}

fn main() -> Fallible<()> {
    let opt: Opt = Opt::parse();
    println!("{opt:?}");

    let version = if let Some(database_path) = opt.database_path {
        let mut conn = Connection::open(database_path)?;
        retrieve_user_version(&mut conn)?
    } else if let Some(version_string) = opt.version_string {
        SQLiteUserVersion::try_from(version_string.parse::<u32>()?)?
    } else {
        unreachable!()
    };

    println!("{version}");

    Ok(())
}

fn retrieve_user_version(conn: &mut Connection) -> Fallible<SQLiteUserVersion> {
    let user_version = conn
        .prepare_cached("PRAGMA user_version")?
        .query([])?
        .next()?
        .context("failed to query the user_version")?
        .get(0)?;

    Ok(user_version)
}

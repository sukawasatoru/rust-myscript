use clap::{ArgGroup, Parser};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::Connection;
use rust_myscript::prelude::*;
use std::convert::{TryFrom, TryInto};
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

pub struct SQLiteUserVersion {
    major: u8,
    minor: u16,
    patch: u8,
}

impl From<(u8, u16, u8)> for SQLiteUserVersion {
    fn from((major, minor, patch): (u8, u16, u8)) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

#[allow(clippy::unusual_byte_groupings)]
impl From<u32> for SQLiteUserVersion {
    fn from(value: u32) -> Self {
        Self {
            major: (value >> 24) as u8,
            minor: ((value & 0b11111111_11111111_00000000) >> 8) as u16,
            patch: (value & 0b11111111) as u8,
        }
    }
}

impl FromSql for SQLiteUserVersion {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let val: i32 = value
            .as_i64()?
            .try_into()
            .map_err(|e| FromSqlError::Other(Box::new(e)))?;
        Ok((val as u32).into())
    }
}

impl std::str::FromStr for SQLiteUserVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v = s.split('.').collect::<Vec<_>>();
        if v.len() != 3 {
            anyhow::bail!("supports semantics version only");
        }

        Ok((v[0].parse()?, v[1].parse()?, v[2].parse()?).into())
    }
}

impl From<&SQLiteUserVersion> for u32 {
    fn from(rhs: &SQLiteUserVersion) -> Self {
        ((rhs.major as u32) << 24) | ((rhs.minor as u32) << 8) | (rhs.patch as u32)
    }
}

impl From<SQLiteUserVersion> for u32 {
    fn from(value: SQLiteUserVersion) -> Self {
        u32::from(&value)
    }
}

impl std::fmt::Display for SQLiteUserVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

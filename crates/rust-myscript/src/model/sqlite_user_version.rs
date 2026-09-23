/*
 * Copyright 2020, 2021, 2022, 2023, 2025 sukawasatoru
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

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use std::cmp::Ordering;

#[derive(Clone, Eq, Debug, PartialEq)]
pub struct SQLiteUserVersion {
    pub major: u8,
    pub minor: u16,
    pub patch: u8,
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

impl From<u32> for SQLiteUserVersion {
    #[allow(clippy::unusual_byte_groupings)]
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

impl PartialOrd<SQLiteUserVersion> for SQLiteUserVersion {
    fn partial_cmp(&self, other: &SQLiteUserVersion) -> Option<Ordering> {
        let major = self.major.cmp(&other.major);
        if major != Ordering::Equal {
            return Some(major);
        }

        let minor = self.minor.cmp(&other.minor);
        if minor != Ordering::Equal {
            return Some(minor);
        }

        let patch = self.patch.cmp(&other.patch);
        if patch != Ordering::Equal {
            return Some(patch);
        }

        Some(Ordering::Equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_user_version() {
        assert_eq!(SQLiteUserVersion::from((1, 2, 3)).to_string(), "1.2.3");
        assert_eq!(
            SQLiteUserVersion::from((255, 65535, 255)).to_string(),
            "255.65535.255"
        );
        assert_eq!(
            u32::from(SQLiteUserVersion::from((255, 65535, 255))),
            u32::MAX
        );

        assert_eq!(
            "1.2.3".parse::<SQLiteUserVersion>().unwrap(),
            SQLiteUserVersion::from((1, 2, 3))
        );
        assert!("".parse::<SQLiteUserVersion>().is_err());
        assert!("0.0.0.0".parse::<SQLiteUserVersion>().is_err());
    }

    #[test]
    fn string_overflow() {
        assert!("256.0.0".parse::<SQLiteUserVersion>().is_err());
        assert!("0.65536.0".parse::<SQLiteUserVersion>().is_err());
        assert!("0.0.256".parse::<SQLiteUserVersion>().is_err());
    }

    #[test]
    fn u32_layout() {
        // Check each field independently, including the two bytes of minor.
        for (packed, fields) in [
            (0x0000_0000, (0, 0, 0)),
            (0x0000_00ff, (0, 0, 255)),
            (0x0000_0100, (0, 1, 0)),
            (0x0000_ff00, (0, 255, 0)),
            (0x0001_0000, (0, 256, 0)),
            (0x00ff_ff00, (0, 65535, 0)),
            (0x0100_0000, (1, 0, 0)),
            (0xff00_0000, (255, 0, 0)),
            (0x0102_0304, (1, 515, 4)),
            (0x7fff_ffff, (127, 65535, 255)),
            (0x8000_0000, (128, 0, 0)),
            (0xffff_ffff, (255, 65535, 255)),
        ] {
            let version = SQLiteUserVersion::from(fields);
            assert_eq!(SQLiteUserVersion::from(packed), version);
            assert_eq!(u32::from(&version), packed);
            assert_eq!(u32::from(version), packed);
        }
    }

    #[test]
    fn u32_boundary_roundtrip() {
        for major in [0, 1, 127, 128, 254, 255] {
            for minor in [0, 1, 255, 256, 32767, 32768, 65534, 65535] {
                for patch in [0, 1, 127, 128, 254, 255] {
                    let version = SQLiteUserVersion::from((major, minor, patch));
                    assert_eq!(SQLiteUserVersion::from(u32::from(&version)), version);
                }
            }
        }
    }

    #[test]
    fn sqlite_signed_user_version() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for (stored, fields) in [
            (0, (0, 0, 0)),
            (i32::MAX, (127, 65535, 255)),
            (i32::MIN, (128, 0, 0)),
            (-1, (255, 65535, 255)),
        ] {
            conn.pragma_update(None, "user_version", stored).unwrap();
            let version: SQLiteUserVersion = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, SQLiteUserVersion::from(fields));
            assert_eq!(u32::from(version) as i32, stored);
        }
    }
}

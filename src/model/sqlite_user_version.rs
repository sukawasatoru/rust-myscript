/*
 * Copyright 2020, 2021, 2022 sukawasatoru
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn sqlite_user_version() {
        assert_eq!(0b11111111_11111111_11111111_11111111u32, u32::MAX);

        assert_eq!(
            (0b11111111u32 << 24) | (0b11111111_11111111u32 << 8) | 0b11111111,
            u32::MAX
        );

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
    fn parse_u32() {
        let orig = SQLiteUserVersion::from((1, 2, 3));
        let orig_u32 = u32::from(orig.clone());
        let orig_u32_version = SQLiteUserVersion::from(orig_u32);

        assert_eq!(orig_u32_version, orig);
    }

    #[test]
    fn parse_u32_max() {
        let orig = SQLiteUserVersion::from((255, 65535, 255));
        let orig_u32 = u32::from(orig.clone());
        let orig_u32_version = SQLiteUserVersion::from(orig_u32);

        assert_eq!(orig_u32_version, orig);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn parse_u32_matrix() {
        let mut futs = futures::stream::FuturesUnordered::new();
        let cpus = num_cpus::get();
        let duration = 255 / cpus;
        let mut count = 0;
        for i in 0..cpus {
            futs.push(tokio::task::spawn(async move {
                let start = count;
                // "cpus + 1" means "=" of "0..=255".
                let end = count + duration + if i == cpus - 1 { 255 % cpus + 1 } else { 0 };

                for major in start..end {
                    for minor in 0..=65535 {
                        for patch in 0..=255 {
                            let major = major as u8;
                            let orig = SQLiteUserVersion::from((major, minor, patch));
                            let orig_u32 = u32::from(orig.clone());
                            let orig_u32_version = SQLiteUserVersion::from(orig_u32);

                            assert_eq!(orig_u32_version, orig);
                        }
                    }
                }
            }));
            count += duration;
        }

        while let Some(data) = futs.next().await {
            if let Err(e) = data {
                dbg!(e);
                assert!(false);
            }
        }
    }
}

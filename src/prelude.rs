pub use anyhow::{Context as _, Result as Fallible, bail, ensure};
pub use tracing::{debug, error, info, info_span, trace, warn};

#[derive(Default)]
pub struct TomlLoader {
    buf: String,
}

impl TomlLoader {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn load<T>(&mut self, path: &std::path::Path) -> Fallible<T>
    where
        T: serde::de::DeserializeOwned,
    {
        use std::io::Read;

        self.buf.clear();
        std::io::BufReader::new(std::fs::File::open(path)?).read_to_string(&mut self.buf)?;
        Ok(toml::from_str(&self.buf)?)
    }
}

pub struct HexFormat<'a>(pub &'a [u8]);

impl std::fmt::Display for HexFormat<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        write!(f, "{:02X?}", self.0[0])?;

        for entry in &self.0[1..self.0.len()] {
            write!(f, ":{entry:02X?}")?;
        }

        Ok(())
    }
}

pub struct StrVisitor;

impl<'de> serde::de::Visitor<'de> for StrVisitor {
    type Value = &'de str;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a borrowed string")
    }

    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v)
    }
}

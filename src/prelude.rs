pub use anyhow::{bail, ensure, Context as _, Result as Fallible};
pub use tracing::{debug, info, trace, warn};

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

impl<'a> std::fmt::Display for HexFormat<'a> {
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

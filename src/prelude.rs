pub use anyhow::Context as _;

pub struct TomlLoader {
    buf: String,
}

impl TomlLoader {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn load<'a, T>(&'a mut self, path: &std::path::Path) -> anyhow::Result<T>
    where
        T: serde::de::Deserialize<'a>,
    {
        use std::io::Read;

        self.buf.clear();
        std::io::BufReader::new(std::fs::File::open(path)?).read_to_string(&mut self.buf)?;
        Ok(toml::from_str::<T>(&self.buf)?)
    }
}

impl Default for TomlLoader {
    fn default() -> Self {
        Self {
            buf: Default::default(),
        }
    }
}

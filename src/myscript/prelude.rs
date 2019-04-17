use failure::Fail;
pub use failure::Fallible;

pub type Result<T> = std::result::Result<T, failure::Error>;

#[derive(Fail, Debug)]
#[fail(display = "Option error")]
pub struct OptionError;

pub trait OkOrErr<T> {
    fn ok_or_err(self) -> Result<T>;
}

impl<T> OkOrErr<T> for Option<T> {
    fn ok_or_err(self) -> Result<T> {
        self.ok_or_else(|| OptionError.into())
    }
}

pub struct TomlLoader {
    buf: String,
}

impl TomlLoader {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn load<'a, T>(&'a mut self, path: &std::path::Path) -> Fallible<T>
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

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

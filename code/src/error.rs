use std::fmt::{Debug, Display, Formatter};

// Error type we map into, when we face one, which does not implement std::error::Error
#[derive(Debug)]
pub struct Error {
    pub msg: String
}
impl<T: Debug> From<veml7700::Error<T>> for Error {
    fn from(value: veml7700::Error<T>) -> Self {
        Error {
            msg: format!("{:?}", value)
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

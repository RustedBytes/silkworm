use thiserror::Error;

#[derive(Error, Debug)]
pub enum SilkwormError {
    #[error("http error: {0}")]
    Http(String),
    #[error("selector error: {0}")]
    Selector(String),
    #[error("spider error: {0}")]
    Spider(String),
    #[error("pipeline error: {0}")]
    Pipeline(String),
    #[error("config error: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type SilkwormResult<T> = Result<T, SilkwormError>;

impl From<reqwest::Error> for SilkwormError {
    fn from(err: reqwest::Error) -> Self {
        SilkwormError::Http(err.to_string())
    }
}

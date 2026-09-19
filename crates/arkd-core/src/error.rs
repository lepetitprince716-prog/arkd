use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    CoreNotFound(String),
    #[error("{0}")]
    CoreLoad(String),
    #[error("{0}")]
    NotConnected(String),
    #[error("{0}")]
    DeviceConnection(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    UnknownTaskType(String),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Busy(String),
    #[error("{0}")]
    PlayTools(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Image(String),
    #[error("{0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;

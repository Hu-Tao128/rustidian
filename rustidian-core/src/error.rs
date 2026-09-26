/// Single error type for the core crate.
///
/// Every fallible function in this crate returns `Result<T, CoreError>`.
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("could not read or write: {0}")]
    Io(#[from] std::io::Error),

    #[error("note '{0}' does not exist")]
    NotFound(String),

    #[error("a note with that name already exists: {0}")]
    NameCollision(String),

    #[error("could not parse configuration: {0}")]
    Config(String),
}

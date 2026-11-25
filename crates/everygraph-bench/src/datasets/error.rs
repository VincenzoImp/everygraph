// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One error type for the dataset tools: a message meant for a person, built up with
//! context as it travels outwards.

use std::fmt;

/// An error of the dataset registry or fetcher, with a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    /// An error with this message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message, without any prefix.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The result type of the dataset tools.
pub type Result<T> = std::result::Result<T, Error>;

/// Adds what was being done to an error from below.
pub(crate) trait Context<T> {
    /// Prefixes the error with `what`.
    fn context(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| Error::new(format!("{}: {e}", what())))
    }
}

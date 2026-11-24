//! Error types for Konami KCE parsing and synthesis.

use core::fmt;

/// Errors returned when parsing Konami KCE soundbanks, sample headers, or audio streams.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer ended before a complete header or payload could be read.
    UnexpectedEof,
    /// Soundbank or sample header field failed structural validation.
    InvalidHeader(&'static str),
    /// Loop start offset exceeded the sample length.
    InvalidLoopRange { loop_start: u32, pcm_length: usize },
    /// Sample index was out of range for the soundbank or pointer table.
    InvalidSample(u32),
    /// Invalid synthesis parameter.
    InvalidParameter(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => {
                write!(f, "Unexpected end of data while parsing Konami KCE stream")
            }
            Self::InvalidHeader(msg) => write!(f, "Invalid Konami KCE header: {msg}"),
            Self::InvalidLoopRange {
                loop_start,
                pcm_length,
            } => write!(
                f,
                "Invalid Konami KCE loop start {loop_start} for {pcm_length}-byte sample"
            ),
            Self::InvalidSample(id) => write!(f, "Invalid Konami KCE sample index {id}"),
            Self::InvalidParameter(msg) => write!(f, "Invalid Konami KCE parameter: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Convenience result alias for `konami-kce`.
pub type Result<T> = core::result::Result<T, Error>;

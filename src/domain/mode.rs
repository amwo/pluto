use std::str::FromStr;

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Observe,
    Paper,
    Live,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Observe => "observe",
            Mode::Paper => "paper",
            Mode::Live => "live",
        }
    }
}

impl FromStr for Mode {
    type Err = ModeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "observe" => Ok(Mode::Observe),
            "paper" => Ok(Mode::Paper),
            "live" => Ok(Mode::Live),
            other => Err(ModeParseError(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid mode: {0}")]
pub struct ModeParseError(String);

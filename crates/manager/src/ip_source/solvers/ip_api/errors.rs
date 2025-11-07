use thiserror::Error;

use super::SourceError;

#[derive(Debug, Error, Clone)]
pub enum IpSolverError {
    #[error("Rate limited by IP solver")]
    RateLimited,
    #[error("Error while getting IP solver response: `{0}`")]
    Other(SourceError),
}

impl From<reqwest::Error> for IpSolverError {
    fn from(value: reqwest::Error) -> Self {
        IpSolverError::Other(SourceError {
            msg: format!("IP Solver failed to resolve: {}", value),
        })
    }
}

impl From<IpSolverError> for SourceError {
    fn from(value: IpSolverError) -> Self {
        SourceError {
            msg: format!("IP API failed to resolve: {}", value),
        }
    }
}

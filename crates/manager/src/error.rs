use std::net::AddrParseError;

use crate::{ip_source::SourceError, svc::FinderError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Service error: `{0}`")]
    Service(FinderError),
    #[error("Invalid IP address: `{0}`")]
    InvalidIpAddress(String),
    #[error("Kube error: `{0}`")]
    Kube(kube::Error),
    #[error("IP source error: `{0}`")]
    IpSource(SourceError),
}

impl From<kube::Error> for Error {
    fn from(value: kube::Error) -> Self {
        Error::Kube(value)
    }
}

impl From<SourceError> for Error {
    fn from(value: SourceError) -> Self {
        Error::IpSource(value)
    }
}

impl From<AddrParseError> for Error {
    fn from(value: AddrParseError) -> Self {
        Error::InvalidIpAddress(value.to_string())
    }
}

impl From<FinderError> for Error {
    fn from(value: FinderError) -> Self {
        Error::Service(value)
    }
}

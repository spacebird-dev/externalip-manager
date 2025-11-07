use std::net::AddrParseError;

use crate::{external_ip_source::IpSourceError, svc::FinderError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Service error: `{0}`")]
    Service(FinderError),
    #[error("Invalid IP address: `{0}`")]
    InvalidIpAddress(String),
    #[error("Kube error: `{0}`")]
    Kube(kube::Error),
    #[error("IP source {name} failed: `{err}`")]
    IPSource { name: String, err: IpSourceError },
}

impl From<kube::Error> for Error {
    fn from(value: kube::Error) -> Self {
        Error::Kube(value)
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

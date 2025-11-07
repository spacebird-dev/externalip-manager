use std::fmt::Debug;

mod registry;
mod solvers;
mod source;

pub use registry::IPSourceRegistry;
pub use source::{AddressKind, ExternalIpSource, ExternalIpSourceKind};

use crate::external_ip_source::solvers::SolverError;

#[derive(Debug, thiserror::Error)]
pub enum IpSourceError {
    #[error("k8s operation failed: `{0}`")]
    Kube(kube::Error),
    #[error("IP solver returned error: `{0}`")]
    Solver(SolverError),
    #[error("IP address source is invalid: `{0}`")]
    Malformed(String),
}

impl From<kube::Error> for IpSourceError {
    fn from(value: kube::Error) -> Self {
        IpSourceError::Kube(value)
    }
}
impl From<SolverError> for IpSourceError {
    fn from(value: SolverError) -> Self {
        IpSourceError::Solver(value)
    }
}

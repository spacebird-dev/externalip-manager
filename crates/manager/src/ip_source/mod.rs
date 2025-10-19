use std::fmt::Debug;

mod registry;
mod solvers;
mod source;

pub use registry::IPSourceRegistry;
pub use source::{AddressKind, ExternalIpSource, ExternalIpSourceKind};

#[derive(Debug, thiserror::Error, Clone)]
#[error("Failed to query address source: {msg}")]
pub struct SourceError {
    pub msg: String,
}
impl SourceError {
    pub fn new(msg: String) -> SourceError {
        SourceError { msg }
    }
}

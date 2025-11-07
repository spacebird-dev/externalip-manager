use std::fmt::Debug;
use std::net::IpAddr;

use async_trait::async_trait;
use my_ip::MyIp;
use reqwest::Client;

use super::{AddressKind, Source, SourceError};

pub use errors::IpSolverError;
pub use solver::IpSolver;

mod errors;
mod ipify;
mod my_ip;
mod solver;

#[async_trait]
trait IpProvider: Send + Sync + Debug {
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        client: &Client,
    ) -> Result<Vec<IpAddr>, IpSolverError>;
}

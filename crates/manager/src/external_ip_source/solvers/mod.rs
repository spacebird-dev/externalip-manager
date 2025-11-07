use std::{fmt::Debug, net::IpAddr};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use thiserror::Error;

use super::AddressKind;

mod dns_hostname;
mod ip_api;
mod load_balancer_ingress;
mod merge;
mod r#static;

pub use dns_hostname::DnsHostname;
pub use ip_api::IpApiSolver;
pub use load_balancer_ingress::LoadBalancerIngress;
pub use merge::Merge;
pub use r#static::Static;

/// A Source provides a list of externalIP addresses to be processed and applied
#[async_trait]
pub trait Solver: Debug + Send + Sync {
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        svc: &Service,
    ) -> Result<Vec<IpAddr>, SolverError>;

    fn kind(&self) -> &'static str;
}

#[derive(Debug, Error)]
#[error("failed to resolve addresses: {reason}")]
pub struct SolverError {
    pub reason: String,
}

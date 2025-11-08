use std::{fmt::Debug, net::IpAddr};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use thiserror::Error;

use crate::{
    crd::v1alpha1,
    external_ip_source::{IpSourceError, registry::SolverRegistry},
};

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
    /// Query this solver for addresses of type `kind` and return the results.
    ///
    /// `svc` refers to the [Service] being currently resolved for solvers that need to access it.
    /// `solvers` is the global map of solvers across all ExternalIP sources, useful for solvers with subsolvers, such as [Merge].
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        svc: &Service,
        solvers: &SolverRegistry,
    ) -> Result<Vec<IpAddr>, SolverError>;
}

#[derive(Debug, Error)]
#[error("failed to resolve addresses: {reason}")]
pub struct SolverError {
    pub reason: String,
}

impl TryFrom<v1alpha1::SolverKind> for Box<dyn Solver> {
    type Error = IpSourceError;

    fn try_from(value: v1alpha1::SolverKind) -> Result<Self, Self::Error> {
        match value {
            v1alpha1::SolverKind::IpAPI(ip_solver) => {
                let boxed: Box<dyn Solver> = Box::new(IpApiSolver::new(ip_solver.provider));
                Ok(boxed)
            }
            v1alpha1::SolverKind::DnsHostname(dns_hostname) => {
                let boxed: Box<dyn Solver> = Box::new(DnsHostname::new(dns_hostname.host.clone()));
                Ok(boxed)
            }
            v1alpha1::SolverKind::LoadBalancerIngress(_) => {
                let boxed: Box<dyn Solver> = Box::new(LoadBalancerIngress::new());
                Ok(boxed)
            }
            v1alpha1::SolverKind::Static(cfg) => {
                let boxed: Box<dyn Solver> = Box::new(Static::new(cfg.addresses.clone()));
                Ok(boxed)
            }
            v1alpha1::SolverKind::Merge(merge_config) => {
                let boxed: Box<dyn Solver> =
                    Box::new(Merge::new(merge_config.partial_solvers.clone())?);
                Ok(boxed)
            }
        }
    }
}

impl TryFrom<&v1alpha1::SolverKind> for Box<dyn Solver> {
    type Error = IpSourceError;

    fn try_from(value: &v1alpha1::SolverKind) -> Result<Self, Self::Error> {
        Box::try_from((*value).clone())
    }
}

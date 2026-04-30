use async_trait::async_trait;
use hickory_resolver::{Resolver, net::runtime::TokioRuntimeProvider};

use k8s_openapi::api::core::v1::Service;
use tracing::instrument;

use crate::external_ip_source::{self, registry::SolverRegistry, solvers::SolverError};

use super::Solver;

#[derive(Debug)]
pub struct DnsHostname {
    host: String,
    resolver: Resolver<TokioRuntimeProvider>,
}

impl DnsHostname {
    pub fn new(host: String) -> DnsHostname {
        DnsHostname {
            host,
            resolver: Resolver::builder_tokio()
                .expect("could not build DNS resolver")
                .build()
                .expect("could not build the resolver"),
        }
    }
}

#[async_trait]
impl Solver for DnsHostname {
    #[instrument]
    async fn get_addresses(
        &mut self,
        kind: external_ip_source::AddressKind,
        _: &Service,
        _: &SolverRegistry,
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        Ok(self
            .resolver
            .lookup_ip(self.host.clone())
            .await
            .map_err(|e| SolverError {
                reason: e.to_string(),
            })?
            .iter()
            .filter(|addr| match kind {
                external_ip_source::AddressKind::IPv4 => addr.is_ipv4(),
                external_ip_source::AddressKind::IPv6 => addr.is_ipv6(),
            })
            .collect())
    }
}

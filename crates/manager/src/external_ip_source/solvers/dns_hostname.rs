use std::net::IpAddr;

use async_trait::async_trait;
use hickory_resolver::{Resolver, config::ResolverConfig, name_server::TokioConnectionProvider};
use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use tracing::instrument;

use crate::external_ip_source::{self, solvers::SolverError};

use super::Solver;

#[derive(Debug)]
pub struct DnsHostname {
    host: String,
    resolver: Resolver<TokioConnectionProvider>,
}

impl DnsHostname {
    pub fn new(host: String) -> DnsHostname {
        DnsHostname {
            host,
            resolver: Resolver::builder_with_config(
                ResolverConfig::default(),
                TokioConnectionProvider::default(),
            )
            .build(),
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
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        match kind {
            external_ip_source::AddressKind::IPv4 => Ok(self
                .resolver
                .ipv4_lookup(self.host.clone())
                .await
                .map_err(|e| SolverError {
                    reason: e.to_string(),
                })?
                .iter()
                .map(|a| IpAddr::V4(a.0))
                .collect_vec()),
            external_ip_source::AddressKind::IPv6 => Ok(self
                .resolver
                .ipv6_lookup(self.host.clone())
                .await
                .map_err(|e| SolverError {
                    reason: e.to_string(),
                })?
                .iter()
                .map(|a| IpAddr::V6(a.0))
                .collect_vec()),
        }
    }

    fn kind(&self) -> &'static str {
        "dnsHostname"
    }
}

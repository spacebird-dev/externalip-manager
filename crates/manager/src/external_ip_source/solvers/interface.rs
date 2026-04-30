use std::net::IpAddr;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use tracing::instrument;

use crate::external_ip_source::{self, registry::SolverRegistry, solvers::SolverError};

use super::Solver;

#[derive(Debug)]
pub struct Interface {
    identifier: Option<String>,
}

impl Interface {
    pub fn new(identifier: Option<String>) -> Interface {
        Interface { identifier }
    }
}

#[async_trait]
impl Solver for Interface {
    #[instrument]
    async fn get_addresses(
        &mut self,
        kind: external_ip_source::AddressKind,
        _: &Service,
        _: &SolverRegistry,
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        Ok(match kind {
            external_ip_source::AddressKind::IPv4 => {
                let filter = |a: &std::net::Ipv4Addr| {
                    !a.is_private()
                        && !a.is_broadcast()
                        && !a.is_loopback()
                        && !a.is_multicast()
                        && !a.is_link_local()
                        && !a.is_unspecified()
                        && !a.is_documentation()
                };
                let addrs = if let Some(netname) = &self.identifier {
                    getifs::interface_by_name(netname)
                        .map_err(|e| SolverError {
                            reason: format!("unable to retrieve network interfaces: {}", e),
                        })?
                        .ok_or(SolverError {
                            reason: format!("interface {} does not exist", netname),
                        })?
                        .ipv4_addrs_by_filter(filter)
                } else {
                    getifs::local_ipv4_addrs_by_filter(filter)
                };
                addrs
                    .map_err(|e| SolverError {
                        reason: format!("unable to retrieve addresses: {}", e),
                    })?
                    .into_iter()
                    .map(|n| IpAddr::V4(n.addr()))
                    .collect()
            }
            external_ip_source::AddressKind::IPv6 => {
                let filter = |a: &std::net::Ipv6Addr| {
                    !a.is_unicast_link_local()
                        && !a.is_loopback()
                        && !a.is_multicast()
                        && !a.is_unique_local()
                        && !a.is_unspecified()
                };
                let addrs = if let Some(netname) = &self.identifier {
                    getifs::interface_by_name(netname)
                        .map_err(|e| SolverError {
                            reason: format!("unable to retrieve network interfaces: {}", e),
                        })?
                        .ok_or(SolverError {
                            reason: format!("interface {} does not exist", netname),
                        })?
                        .ipv6_addrs_by_filter(filter)
                } else {
                    getifs::local_ipv6_addrs_by_filter(filter)
                };
                addrs
                    .map_err(|e| SolverError {
                        reason: format!("unable to retrieve addresses: {}", e),
                    })?
                    .into_iter()
                    .map(|n| IpAddr::V6(n.addr()))
                    .collect()
            }
        })
    }
}

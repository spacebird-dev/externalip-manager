use std::net::IpAddr;

use async_trait::async_trait;
use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use tracing::{instrument, warn};

use crate::external_ip_source::{
    self, AddressKind,
    registry::SolverRegistry,
    solvers::{Solver, SolverError},
};

#[derive(Debug)]
pub struct Static {
    addresses: Vec<IpAddr>,
}

impl Static {
    pub fn new(addresses: Vec<IpAddr>) -> Static {
        Static { addresses }
    }
}

#[async_trait]
impl Solver for Static {
    #[instrument]
    async fn get_addresses(
        &mut self,
        kind: external_ip_source::AddressKind,
        _: &Service,
        _: &SolverRegistry,
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        Ok(self
            .addresses
            .clone()
            .iter()
            .filter_map(|addr| {
                if kind == AddressKind::IPv4 && !addr.is_ipv4() {
                    warn!(msg = "ignoring non-IPv4 address in 'static' IPv4 address source");
                    None
                } else if kind == AddressKind::IPv6 && !addr.is_ipv6() {
                    warn!(msg = "ignoring non-IPv6 address in 'static' IPv6 address source");
                    None
                } else {
                    Some(*addr)
                }
            })
            .collect_vec())
    }
}

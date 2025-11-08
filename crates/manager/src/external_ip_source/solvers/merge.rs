use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use tokio::time::timeout;
use tracing::info;

use crate::{
    crd::v1alpha1::{self, SolverKind},
    external_ip_source::{
        self, AddressKind, IpSourceError,
        registry::SolverRegistry,
        solvers::{Solver, SolverError},
    },
};

#[derive(Debug)]
pub struct Merge {
    partial_solvers: Vec<v1alpha1::PartialSolver>,
}

impl Merge {
    pub fn new(
        partial_solvers: Vec<v1alpha1::PartialSolver>,
    ) -> Result<Merge, external_ip_source::IpSourceError> {
        // nested merges do not make sense
        // we cannot check if the ip address type is correct at this point, but we can make sure that is generically valid at least
        // Ensure that the parts masks combine into a full address
        let mask_sum = partial_solvers
            .iter()
            .map(|ps| match ps.mask {
                IpAddr::V4(ipv4_addr) => u128::from(ipv4_addr.to_bits()),
                IpAddr::V6(ipv6_addr) => ipv6_addr.to_bits(),
            })
            .sum::<u128>();
        if ![u128::from(u32::MAX), u128::MAX].contains(&mask_sum) {
            return Err(IpSourceError::Malformed(format!(
                "merge part netmasks do not combine to full IPv4 address. Got {}",
                Ipv6Addr::from(mask_sum)
            )));
        }
        Ok(Merge { partial_solvers })
    }
}

fn ip_to_u128(addr: &IpAddr) -> u128 {
    match addr {
        IpAddr::V4(ipv4_addr) => u128::from(ipv4_addr.to_bits()),
        IpAddr::V6(ipv6_addr) => ipv6_addr.to_bits(),
    }
}

#[async_trait]
impl Solver for Merge {
    //#[instrument]
    async fn get_addresses(
        &mut self,
        kind: external_ip_source::AddressKind,
        svc: &Service,
        solvers: &SolverRegistry,
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        // Ensure that the part masks are all of the correct ip type.
        // We already know they build a valid IP address thanks to the check in new()
        if kind == AddressKind::IPv4
            && let Some(mismatch) = self
                .partial_solvers
                .iter()
                .find(|ps| !matches!(ps.mask, IpAddr::V4(_)))
        {
            return Err(SolverError {
                reason: format!(
                    "mismatched merge mask IP type. Expected IPv4 masks, got {}",
                    mismatch.mask
                ),
            });
        } else if kind == AddressKind::IPv6
            && let Some(mismatch) = self
                .partial_solvers
                .iter()
                .find(|ps| !matches!(ps.mask, IpAddr::V6(_)))
        {
            return Err(SolverError {
                reason: format!(
                    "mismatched merge mask IP type. Expected IPv6 masks, got {}",
                    mismatch.mask
                ),
            });
        }

        let mut addrs = vec![];
        let mut parts = vec![];
        for ps in &self.partial_solvers {
            let solver = solvers
                .get(&(SolverKind::from(&ps.solver), kind))
                .ok_or(SolverError {
                    reason: format!("solver {:?} not found", ps),
                })?;
            let mut guard = timeout(Duration::from_secs(10), solver.write())
                .await
                .map_err(|_| SolverError {
                    reason: "timed out waiting for solver lock".to_string(),
                })?;
            let addrs_ret = guard.get_addresses(kind, svc, solvers).await?;
            let addr = addrs_ret.last().ok_or(SolverError {
                reason: "merge partialSolver returned no addresses".to_string(),
            })?;
            let part = ip_to_u128(addr) & ip_to_u128(&ps.mask);
            addrs.push(*addr);
            parts.push(part);
        }
        let address: u128 = parts.into_iter().sum();
        let addr = match kind {
            AddressKind::IPv4 => IpAddr::V4(Ipv4Addr::from_bits(
                u32::try_from(address).expect("ipv4 merge type should result in ipv4 address"),
            )),
            AddressKind::IPv6 => IpAddr::V6(Ipv6Addr::from_bits(address)),
        };
        info!(
            msg = "merge: assembled address from parts",
            address_parts = ?addrs,
            ?addr
        );
        Ok(vec![addr])
    }
}

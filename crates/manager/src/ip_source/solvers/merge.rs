use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use async_trait::async_trait;
use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use tracing::{info, instrument};

use crate::{
    crd::v1alpha1::{self},
    ip_source::{
        self, AddressKind, SourceError,
        solvers::{DnsHostname, IpSolver, LoadBalancerIngress, Source, Static},
    },
};

#[derive(Debug)]
struct PartialSolver {
    mask: IpAddr,
    solver: Box<dyn Source>,
}

#[derive(Debug)]
pub struct Merge {
    solvers: Vec<PartialSolver>,
}

impl Merge {
    pub fn new(
        partial_solvers: Vec<v1alpha1::PartialSolver>,
    ) -> Result<Merge, ip_source::SourceError> {
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
            return Err(SourceError {
                msg: format!(
                    "Merge part netmasks do not combine to full IPv4 address. Got {}",
                    mask_sum
                ),
            });
        }
        Ok(Merge {
            solvers: partial_solvers
                .into_iter()
                .map(|sol| PartialSolver {
                    mask: sol.mask,
                    solver: match sol.solver {
                        v1alpha1::PartialSolverKind::IpAPI(ip_solver) => {
                            let boxed: Box<dyn Source> =
                                Box::new(IpSolver::new(ip_solver.provider));
                            boxed
                        }
                        v1alpha1::PartialSolverKind::DnsHostname(dns_hostname) => {
                            let boxed: Box<dyn Source> =
                                Box::new(DnsHostname::new(dns_hostname.host.clone()));
                            boxed
                        }
                        v1alpha1::PartialSolverKind::LoadBalancerIngress(_) => {
                            let boxed: Box<dyn Source> = Box::new(LoadBalancerIngress::new());
                            boxed
                        }
                        v1alpha1::PartialSolverKind::Static(cfg) => {
                            let boxed: Box<dyn Source> =
                                Box::new(Static::new(cfg.addresses.clone()));
                            boxed
                        }
                    },
                })
                .collect_vec(),
        })
    }
}

fn ip_to_u128(addr: &IpAddr) -> u128 {
    match addr {
        IpAddr::V4(ipv4_addr) => u128::from(ipv4_addr.to_bits()),
        IpAddr::V6(ipv6_addr) => ipv6_addr.to_bits(),
    }
}

#[async_trait]
impl Source for Merge {
    #[instrument]
    async fn get_addresses(
        &self,
        kind: ip_source::AddressKind,
        svc: &Service,
    ) -> Result<Vec<std::net::IpAddr>, ip_source::SourceError> {
        // Ensure that the part masks are all of the correct ip type.
        // We already know they build a valid IP address thanks to the check in new()
        if kind == AddressKind::IPv4
            && let Some(mismatch) = self
                .solvers
                .iter()
                .find(|ps| !matches!(ps.mask, IpAddr::V4(_)))
        {
            return Err(SourceError {
                msg: format!(
                    "Mismatched merge mask IP type. Expected IPv4 masks, got {}",
                    mismatch.mask
                ),
            });
        } else if kind == AddressKind::IPv6
            && let Some(mismatch) = self
                .solvers
                .iter()
                .find(|ps| !matches!(ps.mask, IpAddr::V6(_)))
        {
            return Err(SourceError {
                msg: format!(
                    "Mismatched merge mask IP type. Expected IPv6 masks, got {}",
                    mismatch.mask
                ),
            });
        }

        let mut addrs = vec![];
        let mut parts = vec![];
        for ps in &self.solvers {
            let addrs_ret = ps.solver.get_addresses(kind, svc).await?;
            let addr = addrs_ret.last().ok_or(SourceError {
                msg: "merge partialSolver returned no addresses".to_string(),
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

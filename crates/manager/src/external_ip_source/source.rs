use std::{
    fmt::{Debug, Display},
    net::IpAddr,
};

use k8s_openapi::api::core::v1::Service;
use tracing::{debug, error, info, instrument, warn};

use crate::{
    crd::v1alpha1::{self, CLUSTER_EXTERNAL_IP_SOURCE_KIND},
    external_ip_source::{
        self, IpSourceError,
        solvers::{
            DnsHostname, IpApiSolver, LoadBalancerIngress, Merge, Solver, SolverError, Static,
        },
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalIpSourceKind {
    Cluster(String),
}
impl ExternalIpSourceKind {
    #[allow(unused)]
    fn kind(&self) -> String {
        match self {
            ExternalIpSourceKind::Cluster(_) => CLUSTER_EXTERNAL_IP_SOURCE_KIND.to_string(),
        }
    }

    fn name(&self) -> String {
        match self {
            ExternalIpSourceKind::Cluster(name) => name.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct ExternalIpSource {
    kind: ExternalIpSourceKind,
    v4: Option<SolverList>,
    v6: Option<SolverList>,
}
impl ExternalIpSource {
    #[instrument]
    pub async fn query(
        &mut self,
        svc: &Service,
    ) -> Result<Vec<IpAddr>, external_ip_source::IpSourceError> {
        let mut addrs = vec![];
        if let Some(v4) = &mut self.v4 {
            addrs.extend(v4.query(AddressKind::IPv4, svc).await?);
        }
        if let Some(v6) = &mut self.v6 {
            addrs.extend(v6.query(AddressKind::IPv6, svc).await?);
        }
        Ok(addrs)
    }

    pub fn name(&self) -> String {
        self.kind.name()
    }

    #[allow(unused)]
    pub fn kind(&self) -> String {
        self.kind.kind()
    }
}

impl TryFrom<v1alpha1::ClusterExternalIPSource> for ExternalIpSource {
    type Error = IpSourceError;

    fn try_from(value: v1alpha1::ClusterExternalIPSource) -> Result<Self, IpSourceError> {
        if value.spec.ipv4.is_none() && value.spec.ipv6.is_none() {
            return Err(IpSourceError::Malformed(
                "ClusterExternalIpSource needs at least one source block defined".to_string(),
            ));
        }
        Ok(ExternalIpSource {
            kind: ExternalIpSourceKind::Cluster(value.metadata.name.unwrap_or_default()),
            v4: value.spec.ipv4.and_then(|ipv4| {
                SolverList::try_from(ipv4).inspect_err(|e| {
                error!(msg = "unable to create IPv4 solvers for ClusterExternalIpSource", err = ?e);
            }).ok()
            }),
            v6: value.spec.ipv6.and_then(|ipv4| {
                SolverList::try_from(ipv4).inspect_err(|e| {
                error!(msg = "unable to create IPv6 solvers for ClusterExternalIpSource", err = ?e);
            }).ok()
            }),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    IPv4,
    IPv6,
}
impl Display for AddressKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                AddressKind::IPv4 => "IPv4",
                AddressKind::IPv6 => "IPv6",
            }
        )
    }
}

/// A set of solvers that can be queried for external IP addresses
#[derive(Debug)]
struct SolverList {
    solvers: Vec<Box<dyn Solver>>,
    query_mode: QueryMode,
}

impl SolverList {
    #[instrument]
    async fn query(
        &mut self,
        kind: AddressKind,
        svc: &Service,
    ) -> Result<Vec<IpAddr>, IpSourceError> {
        // should be guaranteed from our TryFrom impl
        assert!(
            !self.solvers.is_empty(),
            "at least one solver must be defined"
        );

        let mut collected_addrs: Vec<IpAddr> = vec![];
        let svc_name = format!(
            "{}/{}",
            svc.metadata.namespace.clone().unwrap_or_default(),
            svc.metadata.name.clone().unwrap_or_default()
        );
        for solver in &mut self.solvers {
            match solver.get_addresses(kind, svc).await {
                Ok(addrs) => {
                    if addrs.is_empty() {
                        info!(
                            msg = "solver returned no addresses",
                            svc = svc_name,
                            solver = solver.kind()
                        );
                        continue;
                    }
                    debug!(msg = "retrieved externalIP addresses from solver", svc = svc_name, solver = solver.kind(), addresses = ?addrs);
                    match self.query_mode {
                        QueryMode::FirstFound => {
                            info!(
                                msg = "resolved externalIP addresses for service",
                                svc = svc_name,
                                addresses = ?addrs
                            );
                            return Ok(addrs);
                        }
                        QueryMode::All => {
                            collected_addrs.extend(addrs);
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        msg = "failed to query solver",
                        solver = solver.kind(),
                        err = e.to_string(),
                        svc = svc_name
                    );
                    continue;
                }
            }
        }
        match self.query_mode {
            QueryMode::All if !collected_addrs.is_empty() => {
                info!(
                    msg = "resolved externalIP addresses for service",
                    svc = svc_name,
                    addresses = ?collected_addrs
                );
                Ok(collected_addrs)
            }
            QueryMode::FirstFound | QueryMode::All => Err(IpSourceError::Solver(SolverError {
                reason: "no IP addresses were returned by any source".to_string(),
            })),
        }
    }
}

impl TryFrom<v1alpha1::IpSolversConfig> for SolverList {
    type Error = IpSourceError;

    fn try_from(value: v1alpha1::IpSolversConfig) -> Result<Self, Self::Error> {
        if value.solvers.is_empty() {
            return Err(IpSourceError::Malformed(
                "sources list is empty".to_string(),
            ));
        }
        let sources = value
            .solvers
            .iter()
            .map(|s| match s {
                v1alpha1::SolverKind::IpAPI(ip_solver) => {
                    let boxed: Box<dyn Solver> = Box::new(IpApiSolver::new(ip_solver.provider));
                    Ok(boxed)
                }
                v1alpha1::SolverKind::DnsHostname(dns_hostname) => {
                    let boxed: Box<dyn Solver> =
                        Box::new(DnsHostname::new(dns_hostname.host.clone()));
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
            })
            .collect::<Result<Vec<Box<dyn Solver>>, IpSourceError>>()?;
        Ok(SolverList {
            solvers: sources,
            query_mode: value.query_mode.unwrap_or_default().into(),
        })
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum QueryMode {
    FirstFound,
    All,
}
impl From<v1alpha1::QueryMode> for QueryMode {
    fn from(value: v1alpha1::QueryMode) -> Self {
        match value {
            v1alpha1::QueryMode::FirstFound => QueryMode::FirstFound,
            v1alpha1::QueryMode::All => QueryMode::All,
        }
    }
}

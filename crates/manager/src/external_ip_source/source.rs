use std::{
    fmt::{Debug, Display},
    net::IpAddr,
    time::Duration,
};

use k8s_openapi::api::core::v1::Service;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::{
    crd::v1alpha1::{self, CLUSTER_EXTERNAL_IP_SOURCE_KIND, SolverKind},
    external_ip_source::{self, IpSourceError, registry::SolverRegistry, solvers::SolverError},
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
        &self,
        svc: &Service,
        solvers: &SolverRegistry,
    ) -> Result<Vec<IpAddr>, external_ip_source::IpSourceError> {
        let mut addrs = vec![];
        if let Some(v4) = &self.v4 {
            addrs.extend(v4.query(AddressKind::IPv4, svc, solvers).await?);
        }
        if let Some(v6) = &self.v6 {
            addrs.extend(v6.query(AddressKind::IPv6, svc, solvers).await?);
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
impl TryFrom<&v1alpha1::ClusterExternalIPSource> for ExternalIpSource {
    type Error = IpSourceError;

    fn try_from(value: &v1alpha1::ClusterExternalIPSource) -> Result<Self, Self::Error> {
        ExternalIpSource::try_from((*value).clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    solver_refs: Vec<SolverKind>,
    query_mode: QueryMode,
}

impl SolverList {
    #[instrument]
    async fn query(
        &self,
        kind: AddressKind,
        svc: &Service,
        solvers: &SolverRegistry,
    ) -> Result<Vec<IpAddr>, IpSourceError> {
        // should be guaranteed from our TryFrom impl
        assert!(
            !self.solver_refs.is_empty(),
            "at least one solver must be defined"
        );

        let mut collected_addrs: Vec<IpAddr> = vec![];
        let svc_name = format!(
            "{}/{}",
            svc.metadata.namespace.clone().unwrap_or_default(),
            svc.metadata.name.clone().unwrap_or_default()
        );
        for solv_ref in &self.solver_refs {
            let solver = solvers
                .get(&((*solv_ref).clone(), kind))
                .ok_or(IpSourceError::Solver(SolverError {
                    reason: format!("solver {:?} not found", (solv_ref, kind)),
                }))?;
            let mut guard = timeout(Duration::from_secs(5), solver.write())
                .await
                .map_err(|_| {
                    IpSourceError::Solver(SolverError {
                        reason: "timed out waiting for solver".to_string(),
                    })
                })?;
            match guard.get_addresses(kind, svc, solvers).await {
                Ok(addrs) => {
                    if addrs.is_empty() {
                        info!(
                            msg = "solver returned no addresses",
                            svc = svc_name,
                            solver = ?solv_ref
                        );
                        continue;
                    }
                    debug!(msg = "retrieved externalIP addresses from solver", svc = svc_name, solver = ?solv_ref, addresses = ?addrs);
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
                        solver = ?solv_ref,
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
        Ok(SolverList {
            solver_refs: value.solvers,
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

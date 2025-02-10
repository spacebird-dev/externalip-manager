use std::{fmt::Debug, net::IpAddr};

use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use solvers::{DnsHostname, IpSolver, LoadBalancerIngress, Source};
use tracing::{error, info, instrument};

use crate::crd::v1alpha1::{self, CLUSTER_EXTERNAL_IP_SOURCE_KIND};

mod solvers;

#[derive(Debug, thiserror::Error, Clone)]
#[error("Failed to query address source: {msg}")]
pub struct SourceError {
    pub msg: String,
}
impl SourceError {
    pub fn new(msg: String) -> SourceError {
        SourceError { msg }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalIpSourceKind {
    Cluster(String),
}
impl ExternalIpSourceKind {
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

pub struct ExternalIpSource {
    kind: ExternalIpSourceKind,
    v4: Option<SourceList>,
    v6: Option<SourceList>,
}
impl ExternalIpSource {
    pub async fn query(&self, svc: &Service) -> Result<Vec<IpAddr>, SourceError> {
        let mut addrs = vec![];
        if let Some(v4) = &self.v4 {
            addrs.extend(v4.query(AddressKind::IPv4, svc).await?);
        }
        if let Some(v6) = &self.v6 {
            addrs.extend(v6.query(AddressKind::IPv6, svc).await?);
        }
        Ok(addrs)
    }

    pub fn name(&self) -> String {
        self.kind.name()
    }

    pub fn kind(&self) -> String {
        self.kind.kind()
    }
}

impl TryFrom<v1alpha1::ClusterExternalIPSource> for ExternalIpSource {
    type Error = SourceError;

    fn try_from(value: v1alpha1::ClusterExternalIPSource) -> Result<Self, SourceError> {
        if value.spec.ipv4.is_none() && value.spec.ipv6.is_none() {
            return Err(SourceError {
                msg: "ClusterExternalIpSource needs at least one source block defined".to_string(),
            });
        }
        Ok(ExternalIpSource {
            kind: ExternalIpSourceKind::Cluster(value.metadata.name.unwrap_or_default()),
            v4: value.spec.ipv4.and_then(|ipv4| {
                SourceList::try_from(ipv4).inspect_err(|e| {
                error!(msg = "Unable to create v4 source for ClusterExternalIpSource", err = ?e);
            }).ok()
            }),
            v6: value.spec.ipv6.and_then(|ipv4| {
                SourceList::try_from(ipv4).inspect_err(|e| {
                error!(msg = "Unable to create v6 source for ClusterExternalIpSource", err = ?e);
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

/// A set of sources that can be queried for external IP addresses
#[derive(Debug)]
pub struct SourceList {
    sources: Vec<Box<dyn Source>>,
    query_mode: QueryMode,
}

impl SourceList {
    #[instrument]
    pub async fn query(
        &self,
        kind: AddressKind,
        svc: &Service,
    ) -> Result<Vec<IpAddr>, SourceError> {
        // should be guaranteed from our TryFrom impl
        assert!(
            !self.sources.is_empty(),
            "At least one source must be defined"
        );

        let mut collected_addrs: Vec<IpAddr> = vec![];
        let svc_name = format!(
            "{}/{}",
            svc.metadata.namespace.clone().unwrap_or_default(),
            svc.metadata.name.clone().ok_or(SourceError {
                msg: "annotated service has no name".to_string()
            })?
        );
        for source in &self.sources {
            match source.get_addresses(kind, svc).await {
                Ok(addrs) => {
                    if addrs.is_empty() {
                        info!(msg = "Source did not return and externalIP addresses, skipping", svc = svc_name, source = ?source);
                        continue;
                    }
                    info!(msg = "Retrieved externalIP addresses from source", svc = svc_name, source = ?source, addresses = ?addrs);
                    match self.query_mode {
                        QueryMode::FirstFound => {
                            info!(
                                msg = "Resolved externalIP addresses for service",
                                svc = svc_name,
                                addrs = ?addrs
                            );
                            return Ok(addrs);
                        }
                        QueryMode::All => {
                            collected_addrs.extend(addrs);
                        }
                    }
                }
                Err(e) => {
                    info!(
                        msg = "Failed to query source",
                        source = ?source,
                        err = e.msg,
                        svc = svc_name
                    );
                    continue;
                }
            }
        }
        match self.query_mode {
            QueryMode::All if !collected_addrs.is_empty() => {
                info!(
                    msg = "Resolved externalIP addresses for service",
                    svc = svc_name,
                    addrs = ?collected_addrs
                );
                Ok(collected_addrs)
            }
            QueryMode::FirstFound | QueryMode::All => {
                error!(
                    msg = "No IP addresses were returned by any source, see logs for details",
                    svc = svc_name
                );
                Err(SourceError {
                    msg: "No IP addresses were returned by any source".to_string(),
                })
            }
        }
    }
}

impl TryFrom<v1alpha1::IpSolversConfig> for SourceList {
    type Error = SourceError;

    fn try_from(value: v1alpha1::IpSolversConfig) -> Result<Self, Self::Error> {
        if value.solvers.is_empty() {
            return Err(SourceError {
                msg: "Sources list is empty".to_string(),
            });
        }
        let sources: Vec<Box<dyn Source>> = value
            .solvers
            .iter()
            .map(|s| match s {
                v1alpha1::SolverKind::IpAPI(ip_solver) => {
                    let boxed: Box<dyn Source> = Box::new(IpSolver::new(ip_solver.provider));
                    boxed
                }
                v1alpha1::SolverKind::DnsHostname(dns_hostname) => {
                    let boxed: Box<dyn Source> =
                        Box::new(DnsHostname::new(dns_hostname.host.clone()));
                    boxed
                }
                v1alpha1::SolverKind::LoadBalancerIngress(_) => {
                    let boxed: Box<dyn Source> = Box::new(LoadBalancerIngress::new());
                    boxed
                }
            })
            .collect_vec();
        Ok(SourceList {
            sources,
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

use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use kube::{Api, Client, Resource, api::ListParams, runtime::events::EventType};
use tokio::sync::RwLock;
use tracing::error;

use crate::{
    crd::v1alpha1::{ClusterExternalIPSource, SolverKind},
    events::EventRecorder,
    external_ip_source::{AddressKind, ExternalIpSource, IpSourceError, solvers::Solver},
};

// Solvers are registered globally so that multiple solvers with the same config can be reused for caching.
// Since some solvers like merge call other sub-solvers, the RwLock is needed to ensure consistency + Sync.
// Solvers must not refer to themselves in a nested fashion, else we deadlock. This is ensured by the CRD tree structure.
pub type SolverRegistry = HashMap<(SolverKind, AddressKind), RwLock<Box<dyn Solver>>>;

const REASON_EIP_ERROR: &str = "InvalidIPSource";

pub struct IPSourceRegistry {
    ceips_api: Api<ClusterExternalIPSource>,
    cluster_eip_sources: HashMap<String, ExternalIpSource>,
    solvers: SolverRegistry,
    events: EventRecorder,
}

impl IPSourceRegistry {
    pub async fn new(
        client: Client,
        events: EventRecorder,
    ) -> Result<IPSourceRegistry, IpSourceError> {
        let mut registry = IPSourceRegistry {
            ceips_api: Api::all(client.clone()),
            cluster_eip_sources: HashMap::new(),
            solvers: HashMap::new(),
            events,
        };
        registry.refresh().await?;
        Ok(registry)
    }

    pub async fn refresh(&mut self) -> Result<(), IpSourceError> {
        let cluster_eip_apiobjs = self.ceips_api.list(&ListParams::default()).await?;

        let (ceips_list, errs): (Vec<_>, Vec<_>) = cluster_eip_apiobjs
            .iter()
            .map(|ceips| {
                let ceips_ref = ceips.object_ref(&());
                ExternalIpSource::try_from(ceips).map_err(|e| (e, ceips_ref))
            })
            .partition(Result::is_ok);
        self.cluster_eip_sources = ceips_list
            .into_iter()
            .map(Result::unwrap)
            .map(|ceips| (ceips.name(), ceips))
            .collect();
        let errs = errs.into_iter().map(Result::unwrap_err).collect_vec();
        for (e, ceips_ref) in errs {
            error!(msg = "failed to parse ClusterExternalIPSource", err = ?e, name = ceips_ref.name, namespace = ceips_ref.namespace);
            self.events
                .publish(
                    REASON_EIP_ERROR.to_string(),
                    "ParsingClusterExternalIPSource".to_string(),
                    EventType::Warning,
                    Some(format!("Invalid ClusterExternalIPSource: {e}")),
                    &ceips_ref,
                )
                .await;
        }

        // Populate solver map globally so multiple solvers with the same config reference one solver for caching purposes
        let current_solver_refs = cluster_eip_apiobjs
            .into_iter()
            .flat_map(|ceips_apiobj| {
                let mut solvers = vec![];
                if let Some(ipv4) = ceips_apiobj.spec.ipv4 {
                    solvers.extend(ipv4.solvers.clone().into_iter().flat_map(|s| {
                        match &s {
                            SolverKind::Merge(merge_config) => {
                                let mut solvers = merge_config
                                    .partial_solvers
                                    .iter()
                                    .map(|ps| (SolverKind::from(&ps.solver), AddressKind::IPv4))
                                    .collect_vec();
                                solvers.push((s, AddressKind::IPv4));
                                solvers.into_iter()
                            }
                            _ => vec![(s, AddressKind::IPv4)]
                                .into_iter()
                                .collect_vec()
                                .into_iter(),
                        }
                    }))
                }
                if let Some(ipv6) = ceips_apiobj.spec.ipv6 {
                    solvers.extend(ipv6.solvers.clone().into_iter().flat_map(|s| {
                        match &s {
                            SolverKind::Merge(merge_config) => {
                                let mut solvers = merge_config
                                    .partial_solvers
                                    .iter()
                                    .map(|ps| (SolverKind::from(&ps.solver), AddressKind::IPv6))
                                    .collect_vec();
                                solvers.push((s, AddressKind::IPv6));
                                solvers.into_iter()
                            }
                            _ => vec![(s, AddressKind::IPv6)]
                                .into_iter()
                                .collect_vec()
                                .into_iter(),
                        }
                    }))
                }
                solvers.into_iter()
            })
            .collect::<HashSet<(SolverKind, AddressKind)>>();
        for solver_ref in &current_solver_refs {
            self.solvers
                .entry(solver_ref.clone())
                .or_insert(RwLock::new(solver_ref.0.clone().try_into()?));
        }

        Ok(())
    }

    pub fn get_cluster(&self, name: &str) -> Option<&ExternalIpSource> {
        self.cluster_eip_sources.get(name)
    }

    pub fn solvers(&self) -> &SolverRegistry {
        &self.solvers
    }
}

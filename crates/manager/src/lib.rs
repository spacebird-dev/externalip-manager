use std::collections::HashSet;
use std::hash::RandomState;
use std::net::IpAddr;

use error::Error;
use external_ip_source::ExternalIpSourceKind;
use itertools::Itertools;
use k8s_openapi::api::core::v1::{Service, ServiceSpec};
use kube::api::{ObjectMeta, Patch, PatchParams};
use kube::runtime::events::EventType;
use kube::{Api, Client, Resource};
use svc::{ExternalIpSvc, ServiceFinder};
use tracing::error;
use tracing::{info, instrument};

use crate::events::EventRecorder;
use crate::external_ip_source::IPSourceRegistry;
use crate::svc::FinderError;

pub mod crd;
mod error;
mod events;
mod external_ip_source;
mod svc;

const ACTION_UPDATE_EIPS: &str = "UpdatingExternlIPs";
const MANAGER_ID: &str = "externalip-manager";

pub struct Manager {
    config: ManagerConfig,
    svc_finder: ServiceFinder,
    ip_sources: IPSourceRegistry,
    client: Client,
    events: EventRecorder,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ManagerConfig {
    pub dry_run: bool,
}

impl Manager {
    pub async fn new(config: ManagerConfig) -> Result<Manager, Error> {
        let client = Client::try_default().await?;
        let events = EventRecorder::new(client.clone(), MANAGER_ID.to_string());
        Ok(Manager {
            config,
            svc_finder: ServiceFinder::new(events.clone()).await?,
            client: client.clone(),
            events: events.clone(),
            ip_sources: IPSourceRegistry::new(client.clone(), events.clone())
                .await
                .map_err(|e| Error::IPSource {
                    name: "registry".to_string(),
                    err: e,
                })?,
        })
    }

    #[instrument(skip(self))]
    pub async fn reconcile_svcs(&mut self) -> Result<Vec<Error>, Error> {
        let mut errors = vec![];
        self.ip_sources
            .refresh()
            .await
            .map_err(|e| Error::IPSource {
                name: "registry".to_string(),
                err: e,
            })?;
        let svcs = match self.svc_finder.find_annotated_svcs().await {
            Ok(svc) => svc,
            Err(e) => {
                let err = Error::Kube(e);
                error!(msg = "could not retrieve list of annotated services", err = ?err);
                return Err(err);
            }
        };
        info!(
            msg = format!(
                "found {} services with externalip-manager annotations",
                svcs.len()
            )
        );

        for svc in svcs.iter().filter_map(|svc| svc.as_ref().ok()) {
            let svc_name = format!(
                "{}/{}",
                svc.svc()
                    .metadata
                    .namespace
                    .as_ref()
                    .expect("service should be in namespace"),
                svc.svc()
                    .metadata
                    .name
                    .as_ref()
                    .expect("service should have name")
            );
            if let Err(e) = self.reconcile_svc(svc).await {
                error!(msg = "failed to reconcile service", svc = svc_name, err = ?e);
                errors.push(e);
            }
        }

        Ok(errors)
    }

    #[instrument(skip(self))]
    async fn reconcile_svc(&mut self, svc: &ExternalIpSvc) -> Result<(), Error> {
        let svc_name = svc.svc().metadata.name.clone().unwrap_or_default();
        let svc_namespace = svc.svc().metadata.namespace.clone().unwrap_or_default();
        let svc_id = format!("{}/{}", svc_namespace, svc_name);
        info!(msg = "processing service", service = svc_id);

        let current_ips: Vec<IpAddr> = match svc
            .svc()
            .spec
            .as_ref()
            .and_then(|spec| spec.external_ips.clone())
            .unwrap_or_default()
            .iter()
            .map(|addr_string| addr_string.parse::<IpAddr>().map_err(Error::from))
            .collect()
        {
            Ok(ips) => ips,
            Err(e) => {
                error!(msg = "service has invalid ExternalIP entries", e = ?e);
                self.events
                    .publish(
                        "InvalidExternalIP".to_string(),
                        ACTION_UPDATE_EIPS.to_string(),
                        EventType::Warning,
                        Some("service has invalid externalIP entries".to_string()),
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                return Err(e);
            }
        };

        let resolved_ips = self.resolve_svc_extipsource_addresses(svc).await?;
        let current_ip_set: HashSet<IpAddr, RandomState> = HashSet::from_iter(current_ips);
        let new_ip_set: HashSet<IpAddr, RandomState> = HashSet::from_iter(resolved_ips);
        if current_ip_set == new_ip_set {
            info!(msg = "service externalIP field already up to date", svc = svc_id, addresses = ?current_ip_set);
            return Ok(());
        }

        if self.config.dry_run {
            info!(msg = "externalIP mismatch for service, not applying changes in dry-run mode", svc = svc_id, current_addresses = ?current_ip_set, new_addresses = ?new_ip_set);
            return Ok(());
        } else {
            info!(msg = "externalIP mismatch for service, updating", svc = svc_id, current_addresses = ?current_ip_set, new_addresses = ?new_ip_set);
        }

        self.update_svc_addresses(svc, new_ip_set.into_iter())
            .await?;

        Ok(())
    }

    async fn resolve_svc_extipsource_addresses(
        &mut self,
        svc: &ExternalIpSvc,
    ) -> Result<Vec<IpAddr>, Error> {
        let ip_source = match svc.ip_source() {
            ExternalIpSourceKind::Cluster(ceips) => self.ip_sources.get_cluster(ceips).ok_or(ceips),
        };
        let ip_source = match ip_source {
            Ok(eips) => eips,
            Err(eips) => {
                self.events
                    .publish(
                        "UnknownExternalIPSource".to_string(),
                        ACTION_UPDATE_EIPS.to_string(),
                        EventType::Warning,
                        Some(format!("Could not find ExternalIPSource {eips}")),
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                return Err(Error::Service(FinderError {
                    msg: format!("could not find ExternalIPSource {eips}"),
                }));
            }
        };

        match ip_source.query(svc.svc(), self.ip_sources.solvers()).await {
            Ok(ips) => Ok(ips),
            Err(e) => {
                self.events
                    .publish(
                        "FailedExternalIPLookup".to_string(),
                        ACTION_UPDATE_EIPS.to_string(),
                        EventType::Warning,
                        Some(format!("Failed to query external IP addresses: {}", e)),
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                Err(Error::IPSource {
                    name: ip_source.name(),
                    err: e,
                })
            }
        }
    }

    async fn update_svc_addresses(
        &self,
        svc: &ExternalIpSvc,
        addresses: impl Iterator<Item = IpAddr>,
    ) -> Result<(), Error> {
        let address_strings = addresses.map(|addr| addr.to_string()).collect_vec();
        let svc_name = svc.svc().metadata.name.clone().unwrap_or_default();
        let svc_namespace = svc.svc().metadata.namespace.clone().unwrap_or_default();
        let svc_id = format!("{}/{}", svc_name, svc_namespace);

        let api: Api<Service> = Api::namespaced(self.client.clone(), &svc_namespace);
        match api
            .patch(
                &svc_name,
                &PatchParams::apply(MANAGER_ID),
                &Patch::Merge(Service {
                    metadata: ObjectMeta::default(),
                    spec: Some(ServiceSpec {
                        external_ips: Some(address_strings.clone()),
                        ..Default::default()
                    }),
                    status: None,
                }),
            )
            .await
        {
            Ok(_) => {
                info!(msg = "service updated", svc = svc_id, ?address_strings);
                self.events
                    .publish(
                        "ExternalIPsUpdated".to_string(),
                        ACTION_UPDATE_EIPS.to_string(),
                        EventType::Normal,
                        None,
                        &svc.svc().object_ref(&()),
                    )
                    .await;
            }
            Err(e) => {
                error!(msg = "failed to update service", svc = svc_id, err = ?e);
                return Err(e.into());
            }
        };
        Ok(())
    }
}

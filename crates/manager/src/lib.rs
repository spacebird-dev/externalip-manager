use std::collections::HashSet;
use std::hash::RandomState;
use std::net::IpAddr;

use crd::v1alpha1::ClusterExternalIPSource;
use error::Error;
use ip_source::{ExternalIpSource, ExternalIpSourceKind};
use itertools::Itertools;
use k8s_openapi::api::core::v1::{ObjectReference, Service, ServiceSpec};
use kube::api::{ObjectMeta, Patch, PatchParams};
use kube::runtime::events::{Event, EventType, Recorder, Reporter};
use kube::{Api, Client, Resource};
use svc::{ExternalIpSvc, ServiceFinder};
use tracing::{error, warn};
use tracing::{info, instrument};

pub mod crd;
mod error;
mod ip_source;
mod svc;

const MANAGER_ID: &str = "externalip-manager";
const ACTION_ID_RECONCILE: &str = "updatingExternalIPs";

pub struct Manager {
    config: ManagerConfig,
    svc_finder: ServiceFinder,
    client: Client,
    recorder: Recorder,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ManagerConfig {
    pub dry_run: bool,
}

impl Manager {
    pub async fn new(config: ManagerConfig) -> Result<Manager, Error> {
        let client = Client::try_default().await?;
        Ok(Manager {
            config,
            svc_finder: ServiceFinder::new().await?,
            client: client.clone(),
            recorder: Recorder::new(
                client.clone(),
                Reporter {
                    controller: MANAGER_ID.to_string(),
                    instance: None,
                },
            ),
        })
    }

    #[instrument(skip(self))]
    pub async fn reconcile_svcs(&self) -> Result<Vec<Error>, Error> {
        let mut errors = vec![];
        let svcs = match self.svc_finder.find_annotated_svcs().await {
            Ok(svc) => svc,
            Err(e) => {
                let err = Error::Kube(e);
                error!(msg = "Could not retrieve list of annotated services", err = ?err);
                return Err(err);
            }
        };
        info!(
            msg = format!(
                "Found {} services with externalip-manager annotations",
                svcs.len()
            )
        );

        for err in svcs.iter().filter_map(|svc| svc.as_ref().err()) {
            warn!(msg = "Failed to process annotated service", err = ?err);
            match err {
                svc::FinderError::ConflictingAnnotations {
                    name,
                    namespace,
                    annotations,
                } => {
                    let api: Api<Service> = Api::namespaced(self.client.clone(), namespace);
                    if let Ok(svc) = api.get(name).await {
                        self.publish_event_reconcile(
                            EventType::Warning,
                            "failingExternalIPResolution".to_string(),
                            Some(format!("Annotations are conflicting: {:?}", annotations)),
                            &svc.object_ref(&()),
                        )
                        .await;
                    }
                }
            }
            errors.push(Error::from(err.clone()));
            continue;
        }

        for svc in svcs.iter().filter_map(|svc| svc.as_ref().ok()) {
            if let Err(e) = self.reconcile_svc(svc).await {
                errors.push(e);
            }
        }

        Ok(errors)
    }

    #[instrument(skip(self))]
    async fn reconcile_svc(&self, svc: &ExternalIpSvc) -> Result<(), Error> {
        let svc_name = svc.svc().metadata.name.clone().unwrap_or_default();
        let svc_namespace = svc.svc().metadata.namespace.clone().unwrap_or_default();
        let svc_id = format!("{}/{}", svc_name, svc_namespace);
        info!(msg = "Processing service", service = svc_id);

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
                error!(msg = "Service has invalid ExternalIP addresses", e = ?e);
                self.publish_event_reconcile(
                    EventType::Warning,
                    "invalidExternalIPAddresses".to_string(),
                    Some("Service has invalid externalIP entries".to_string()),
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
            info!(msg = "Service externalIP field already up to date", svc = svc_id, addresses = ?current_ip_set);
        } else {
            info!(msg = "ExternalIP mismatch for service, updating", svc = svc_id, current_addresses = ?current_ip_set, new_addresses = ?new_ip_set);
        }

        if self.config.dry_run {
            info!(msg = "Not applying changes in dry-run mode");
        }

        self.update_svc_addresses(svc, new_ip_set.into_iter())
            .await?;

        Ok(())
    }

    async fn resolve_svc_extipsource_addresses(
        &self,
        svc: &ExternalIpSvc,
    ) -> Result<Vec<IpAddr>, Error> {
        let ip_source: ExternalIpSource = match svc.ip_source() {
            ExternalIpSourceKind::Cluster(cips_name) => {
                let ceips_api: Api<ClusterExternalIPSource> = Api::all(self.client.clone());
                let ceips = match ceips_api.get(cips_name).await {
                    Ok(cips) => cips,
                    Err(e) => {
                        error!(msg = "Could not retrieve ClusterExternalIPSource", name = cips_name, err = ?e);
                        self.publish_event_reconcile(
                            EventType::Warning,
                            "failingExternalIPSource".to_string(),
                            Some(format!("Could not retrieve ClusterExternalIPSource: {}", e)),
                            &svc.svc().object_ref(&()),
                        )
                        .await;
                        return Err(e.into());
                    }
                };
                let ceips_ref = ceips.object_ref(&());
                match ExternalIpSource::try_from(ceips) {
                    Ok(eips) => eips,
                    Err(e) => {
                        error!(msg = "Unable to use ClusterExternalIPSource", name = cips_name, err = ?e);
                        self.publish_event_reconcile(
                            EventType::Warning,
                            "failingSourceValidation".to_string(),
                            Some(format!("Source is invalid: {}", e)),
                            &ceips_ref,
                        )
                        .await;
                        return Err(e.into());
                    }
                }
            }
        };

        match ip_source.query(svc.svc()).await {
            Ok(ips) => Ok(ips),
            Err(e) => {
                let svc_name = svc.svc().metadata.name.clone().unwrap_or_default();
                let svc_namespace = svc.svc().metadata.namespace.clone().unwrap_or_default();
                let svc_id = format!("{}/{}", svc_name, svc_namespace);
                error!(
                    msg = "Failed to query external IP addresses for service",
                    svc = svc_id,
                    address_source_kind = ip_source.kind(),
                    address_source_name = ip_source.name()
                );
                self.publish_event_reconcile(
                    EventType::Warning,
                    "failingExternalIPQuery".to_string(),
                    Some(format!("Failed to query external IP addresses: {}", e)),
                    &svc.svc().object_ref(&()),
                )
                .await;
                Err(e.into())
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
                info!(msg = "Service updated", svc = svc_id, ?address_strings);
                self.publish_event_reconcile(
                    EventType::Normal,
                    "externalIPsUpdated".to_string(),
                    None,
                    &svc.svc().object_ref(&()),
                )
                .await;
            }
            Err(e) => {
                error!(msg = "Failed to update service", svc = svc_id, err = ?e);
                return Err(e.into());
            }
        };
        Ok(())
    }

    async fn publish_event_reconcile(
        &self,
        type_: EventType,
        reason: String,
        note: Option<String>,
        object_ref: &ObjectReference,
    ) {
        if let Err(e) = self
            .recorder
            .publish(
                &Event {
                    type_,
                    reason: reason.clone(),
                    note: note.clone(),
                    action: ACTION_ID_RECONCILE.to_string(),
                    secondary: None,
                },
                object_ref,
            )
            .await
        {
            warn!(msg = "Failed to publish event for failing service", err = ?e, event = note, reason = reason)
        }
    }
}

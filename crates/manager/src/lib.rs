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
use svc::ServiceFinder;
use tracing::{error, warn};
use tracing::{info, instrument};

pub mod crd;
mod error;
mod ip_source;
mod svc;

const MANAGER_ID: &str = "externalip-manager";

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
    pub async fn reconcile(&self) -> Result<Vec<Error>, Error> {
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
                        self.publish_event(
                            &Event {
                                type_: EventType::Warning,
                                reason: "failingExternalIPResolution".to_string(),
                                note: Some(format!(
                                    "Annotations are conflicting: {:?}",
                                    annotations
                                )),
                                action: "resolvingExternalIP".to_string(),
                                secondary: None,
                            },
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
                    self.publish_event(
                        &Event {
                            type_: EventType::Warning,
                            reason: "invalidExternalIPAddresses".to_string(),
                            note: Some("Service has invalid externalIP entries".to_string()),
                            action: "resolvingExternalIP".to_string(),
                            secondary: None,
                        },
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                    errors.push(e);
                    continue;
                }
            };

            let ip_source: ExternalIpSource = match svc.ip_source() {
                ExternalIpSourceKind::Cluster(cips_name) => {
                    let ceips_api: Api<ClusterExternalIPSource> = Api::all(self.client.clone());
                    let ceips = match ceips_api.get(cips_name).await {
                        Ok(cips) => cips,
                        Err(e) => {
                            error!(msg = "Could not retrieve ClusterExternalIPSource", name = cips_name, err = ?e);
                            self.publish_event(
                                &Event {
                                    type_: EventType::Warning,
                                    reason: "failingExternalIPSource".to_string(),
                                    note: Some(format!(
                                        "Could not retrieve ClusterExternalIPSource: {}",
                                        e
                                    )),
                                    action: "clusterExternalIPSourceValidation".to_string(),
                                    secondary: None,
                                },
                                &svc.svc().object_ref(&()),
                            )
                            .await;
                            errors.push(Error::from(e));
                            continue;
                        }
                    };
                    let ceips_ref = ceips.object_ref(&());
                    match ExternalIpSource::try_from(ceips) {
                        Ok(eips) => eips,
                        Err(e) => {
                            error!(msg = "Unable to use ClusterExternalIPSource", name = cips_name, err = ?e);
                            self.publish_event(
                                &Event {
                                    type_: EventType::Warning,
                                    reason: "failingSourceValidation".to_string(),
                                    note: Some(format!("Source is invalid: {}", e)),
                                    action: "clusterExternalIPSourceValidation".to_string(),
                                    secondary: None,
                                },
                                &ceips_ref,
                            )
                            .await;
                            errors.push(Error::from(e));
                            continue;
                        }
                    }
                }
            };

            let resolved_ips = match ip_source.query(svc.svc()).await {
                Ok(ips) => ips,
                Err(e) => {
                    error!(
                        msg = "Failed to query external IP addresses for service",
                        svc = svc_id,
                        address_source_kind = ip_source.kind(),
                        address_source_name = ip_source.name()
                    );
                    self.publish_event(
                        &Event {
                            type_: EventType::Warning,
                            reason: "failingExternalIPQuery".to_string(),
                            note: Some(format!("Failed to query external IP addresses: {}", e)),
                            action: "resolvingExternalIP".to_string(),
                            secondary: None,
                        },
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                    errors.push(Error::from(e));
                    continue;
                }
            };

            let current_ip_set: HashSet<&IpAddr, RandomState> = HashSet::from_iter(&current_ips);
            let new_ip_set: HashSet<&IpAddr, RandomState> = HashSet::from_iter(&resolved_ips);
            if current_ip_set == new_ip_set {
                info!(msg = "Service externalIP field already up to date", svc = svc_id, addresses = ?current_ip_set);
                continue;
            } else {
                info!(msg = "ExternalIP mismatch for service, updating", svc = svc_id, current_addresses = ?current_ip_set, new_addresses = ?new_ip_set);
            }

            if self.config.dry_run {
                info!(msg = "Not applying changes in dry-run mode");
                continue;
            }

            let resolved_ip_strings = new_ip_set.iter().map(|addr| addr.to_string()).collect_vec();
            let api: Api<Service> = Api::namespaced(self.client.clone(), &svc_namespace);
            match api
                .patch(
                    &svc_name,
                    &PatchParams::apply(MANAGER_ID),
                    &Patch::Merge(Service {
                        metadata: ObjectMeta::default(),
                        spec: Some(ServiceSpec {
                            external_ips: Some(resolved_ip_strings),
                            ..Default::default()
                        }),
                        status: None,
                    }),
                )
                .await
            {
                Ok(_) => {
                    info!(msg = "Service updated", svc = svc_id, addresses = ?new_ip_set );
                    self.publish_event(
                        &Event {
                            type_: EventType::Normal,
                            reason: "externalIPsUpdated".to_string(),
                            note: None,
                            action: "resolvingExternalIP".to_string(),
                            secondary: None,
                        },
                        &svc.svc().object_ref(&()),
                    )
                    .await;
                }
                Err(e) => {
                    error!(msg = "Failed to update service", svc = svc_id, err = ?e);
                    errors.push(Error::from(e));
                }
            };
        }

        Ok(errors)
    }

    async fn publish_event(&self, event: &Event, object_ref: &ObjectReference) {
        if let Err(e) = self.recorder.publish(event, object_ref).await {
            warn!(msg = "Failed to publish event for failing service", err = ?e, event = event.note)
        }
    }
}

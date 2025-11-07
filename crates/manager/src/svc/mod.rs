use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use kube::{Api, Client, api::ListParams};
use tracing::{info, instrument};

use crate::{events::EventRecorder, external_ip_source::ExternalIpSourceKind};

const ANNOTATION_CLUSTER_EXTERNAL_IP_SOURCE: &str =
    "externalip.spacebird.dev/cluster-external-ip-source";

pub struct ServiceFinder {
    svc_api: Api<Service>,
    #[allow(dead_code)]
    events: EventRecorder,
}

impl ServiceFinder {
    pub async fn new(events: EventRecorder) -> Result<ServiceFinder, kube::Error> {
        Ok(ServiceFinder {
            svc_api: Api::all(Client::try_default().await?),
            events,
        })
    }

    #[instrument(skip(self))]
    pub async fn find_annotated_svcs(
        &self,
    ) -> Result<Vec<Result<ExternalIpSvc, FinderError>>, kube::Error> {
        Ok(self
            .svc_api
            .list(&ListParams::default())
            .await?
            .items
            .iter()
            .filter_map(|svc| {
                let Some(annotations) = &svc.metadata.annotations else {
                    return None;
                };
                let extip_cluster_source = annotations.get(ANNOTATION_CLUSTER_EXTERNAL_IP_SOURCE);
                // grab more annotations here in the future

                if let Some(source) = extip_cluster_source {
                    info!(
                        msg = "found service with cluster-external-ip-source annotation",
                        svc = svc.metadata.name,
                        namespace = svc.metadata.namespace
                    );
                    return Some(Ok(ExternalIpSvc {
                        svc: svc.clone(),
                        source: ExternalIpSourceKind::Cluster(source.to_owned()),
                    }));
                }
                None
            })
            .collect_vec())
    }
}

#[derive(Debug)]
pub struct ExternalIpSvc {
    svc: Service,
    source: ExternalIpSourceKind,
}
impl ExternalIpSvc {
    pub fn svc(&self) -> &Service {
        &self.svc
    }

    pub fn ip_source(&self) -> &ExternalIpSourceKind {
        &self.source
    }
}

#[derive(thiserror::Error, Debug, Clone)]
#[error("Failed to process service: {msg}")]
pub struct FinderError {
    pub msg: String,
}

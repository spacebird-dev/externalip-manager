use std::collections::HashMap;

use itertools::Itertools;
use kube::{Api, Client, Resource, api::ListParams, runtime::events::EventType};
use tracing::error;

use crate::{
    crd::v1alpha1::ClusterExternalIPSource, events::EventRecorder,
    external_ip_source::ExternalIpSource,
};

const REASON_EIP_ERROR: &str = "InvalidIPSource";

pub struct IPSourceRegistry {
    ceips_api: Api<ClusterExternalIPSource>,
    ceips: HashMap<String, ExternalIpSource>,
    events: EventRecorder,
}

impl IPSourceRegistry {
    pub async fn new(
        client: Client,
        events: EventRecorder,
    ) -> Result<IPSourceRegistry, kube::Error> {
        let mut registry = IPSourceRegistry {
            ceips_api: Api::all(client.clone()),
            ceips: HashMap::new(),
            events,
        };
        registry.refresh().await?;
        Ok(registry)
    }

    pub async fn refresh(&mut self) -> Result<(), kube::Error> {
        let (ceips, errs): (Vec<_>, Vec<_>) = self
            .ceips_api
            .list(&ListParams::default())
            .await?
            .into_iter()
            .map(|ceips| {
                let ceips_ref = ceips.object_ref(&());
                ExternalIpSource::try_from(ceips).map_err(|e| (e, ceips_ref))
            })
            .partition(Result::is_ok);
        self.ceips = ceips
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
        Ok(())
    }

    pub fn get_cluster(&mut self, name: &str) -> Option<&mut ExternalIpSource> {
        self.ceips.get_mut(name)
    }
}

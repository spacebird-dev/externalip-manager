use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use async_trait::async_trait;
use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use tracing::instrument;

use super::{Source, SourceError};
use crate::ip_source;

#[derive(Debug)]
pub struct LoadBalancerIngress {}

impl LoadBalancerIngress {
    pub fn new() -> LoadBalancerIngress {
        LoadBalancerIngress {}
    }
}

#[async_trait]
impl Source for LoadBalancerIngress {
    #[instrument]
    async fn get_addresses(
        &self,
        kind: ip_source::AddressKind,
        svc: &Service,
    ) -> Result<Vec<std::net::IpAddr>, SourceError> {
        Ok(svc
            .clone()
            .status
            .ok_or(SourceError::new("no status field on service".to_string()))?
            .load_balancer
            .ok_or(SourceError::new(
                "no status.loadBalancer field on service".to_string(),
            ))?
            .ingress
            .ok_or(SourceError::new(
                "no status.loadBalancer.ingress field on service".to_string(),
            ))?
            .iter()
            .filter_map(|addr| {
                addr.clone().ip.and_then(|addr_string| match kind {
                    ip_source::AddressKind::IPv4 => {
                        addr_string.parse::<Ipv4Addr>().ok().map(IpAddr::V4)
                    }
                    ip_source::AddressKind::IPv6 => {
                        addr_string.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
                    }
                })
            })
            .collect_vec())
    }
}

impl From<kube::Error> for SourceError {
    fn from(value: kube::Error) -> Self {
        SourceError {
            msg: value.to_string(),
        }
    }
}

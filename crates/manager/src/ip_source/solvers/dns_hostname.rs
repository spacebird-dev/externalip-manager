use std::net::IpAddr;

use async_trait::async_trait;
use hickory_resolver::{
    ResolveError, Resolver, config::ResolverConfig, name_server::TokioConnectionProvider,
};
use itertools::Itertools;
use k8s_openapi::api::core::v1::Service;
use tracing::instrument;

use crate::ip_source;

use super::{Source, SourceError};

#[derive(Debug)]
pub struct DnsHostname {
    host: String,
    resolver: Resolver<TokioConnectionProvider>,
}

impl DnsHostname {
    pub fn new(host: String) -> DnsHostname {
        DnsHostname {
            host,
            resolver: Resolver::builder_with_config(
                ResolverConfig::default(),
                TokioConnectionProvider::default(),
            )
            .build(),
        }
    }
}

#[async_trait]
impl Source for DnsHostname {
    #[instrument]
    async fn get_addresses(
        &self,
        kind: ip_source::AddressKind,
        _: &Service,
    ) -> Result<Vec<std::net::IpAddr>, ip_source::SourceError> {
        match kind {
            ip_source::AddressKind::IPv4 => Ok(self
                .resolver
                .ipv4_lookup(self.host.clone())
                .await?
                .iter()
                .map(|a| IpAddr::V4(a.0))
                .collect_vec()),
            ip_source::AddressKind::IPv6 => Ok(self
                .resolver
                .ipv6_lookup(self.host.clone())
                .await?
                .iter()
                .map(|a| IpAddr::V6(a.0))
                .collect_vec()),
        }
    }
}

impl From<ResolveError> for SourceError {
    fn from(value: ResolveError) -> Self {
        SourceError {
            msg: value.to_string(),
        }
    }
}

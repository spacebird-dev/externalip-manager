use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::instrument;

use crate::ip_source::{
    AddressKind,
    solvers::ip_api::{IpProvider, IpSolverError},
};

const IPIFY_URL_V4: &str = "https://api.ipify.org?format=json";
const IPIFY_URL_V6: &str = "https://api6.ipify.org?format=json";

#[derive(Deserialize, Debug, Clone, Copy)]
#[allow(dead_code)]
struct IpifyResponse {
    ip: IpAddr,
}

#[derive(Debug)]
pub struct Ipify {}
impl Ipify {
    pub fn new() -> Ipify {
        Ipify {}
    }
}
#[async_trait]
impl IpProvider for Ipify {
    #[instrument(skip(self, client))]
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        client: &Client,
    ) -> Result<Vec<std::net::IpAddr>, IpSolverError> {
        let res = client
            .get(match kind {
                AddressKind::IPv4 => IPIFY_URL_V4,
                AddressKind::IPv6 => IPIFY_URL_V6,
            })
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(IpSolverError::RateLimited);
        }
        let res = res.error_for_status()?.json::<IpifyResponse>().await?;
        Ok(vec![res.ip])
    }
}

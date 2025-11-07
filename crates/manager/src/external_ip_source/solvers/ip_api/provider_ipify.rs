use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::instrument;

use crate::external_ip_source::{
    AddressKind,
    solvers::ip_api::{IpProvider, IpProviderError, IpProviderResponse},
};

const IPIFY_URL_V4: &str = "https://api.ipify.org?format=json";
const IPIFY_URL_V6: &str = "https://api6.ipify.org?format=json";
const CACHE_DURATION: Duration = Duration::from_secs(300);

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
    async fn get_addresses(&mut self, kind: AddressKind, client: &Client) -> IpProviderResponse {
        let res = match client
            .get(match kind {
                AddressKind::IPv4 => IPIFY_URL_V4,
                AddressKind::IPv6 => IPIFY_URL_V6,
            })
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(res) => res,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            return IpProviderResponse::new(
                CACHE_DURATION,
                Err(IpProviderError::RateLimited {
                    remaining: CACHE_DURATION,
                }),
            );
        }
        let body = match res.error_for_status() {
            Ok(body) => body,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        let response = match body.json::<IpifyResponse>().await {
            Ok(resp) => resp,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        IpProviderResponse::new(CACHE_DURATION, Ok(vec![response.ip]))
    }
}

use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::instrument;

use crate::external_ip_source::{
    AddressKind,
    solvers::ip_api::{IpProvider, IpProviderError, IpProviderResponse},
};

const MY_IP_URL_V4: &str = "https://api4.my-ip.io/v2/ip.json";
const MY_IP_URL_V6: &str = "https://api6.my-ip.io/v2/ip.json";
const CACHE_DURATION: Duration = Duration::from_secs(900);

#[derive(Deserialize, Debug, Clone, Copy)]
enum MyIpType {
    IPv4,
    IPv6,
}
#[derive(Deserialize, Debug, Clone, Copy)]
#[allow(dead_code)]
struct MyIpResponse {
    success: bool,
    ip: IpAddr,
    r#type: MyIpType,
}

#[derive(Debug)]
pub struct MyIp {}
impl MyIp {
    pub fn new() -> MyIp {
        MyIp {}
    }
}
#[async_trait]
impl IpProvider for MyIp {
    #[instrument(skip(self, client))]
    async fn get_addresses(&mut self, kind: AddressKind, client: &Client) -> IpProviderResponse {
        let res = match client
            .get(match kind {
                AddressKind::IPv4 => MY_IP_URL_V4,
                AddressKind::IPv6 => MY_IP_URL_V6,
            })
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(res) => res,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            return IpProviderResponse::new(CACHE_DURATION, Err(IpProviderError::RateLimited));
        }
        let body = match res.error_for_status() {
            Ok(body) => body,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        let response = match body.json::<MyIpResponse>().await {
            Ok(resp) => resp,
            Err(e) => return IpProviderResponse::new(CACHE_DURATION, Err(e.into())),
        };
        IpProviderResponse::new(CACHE_DURATION, Ok(vec![response.ip]))
    }
}

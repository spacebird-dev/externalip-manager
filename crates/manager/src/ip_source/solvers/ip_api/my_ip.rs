use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use super::IpProvider;
use crate::ip_source::{AddressKind, SourceError};

const MY_IP_URL_V4: &str = "https://api4.my-ip.io/v2/ip.json";
const MY_IP_URL_V6: &str = "https://api6.my-ip.io/v2/ip.json";

#[derive(Deserialize)]
enum MyIpType {
    IPv4,
    IPv6,
}
#[derive(Deserialize)]
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
    async fn get_addresses(
        &self,
        kind: AddressKind,
        client: &Client,
    ) -> Result<Vec<std::net::IpAddr>, SourceError> {
        Ok(vec![
            client
                .get(match kind {
                    AddressKind::IPv4 => MY_IP_URL_V4,
                    AddressKind::IPv6 => MY_IP_URL_V6,
                })
                .timeout(Duration::from_secs(10))
                .send()
                .await?
                .json::<MyIpResponse>()
                .await?
                .ip,
        ])
    }
}

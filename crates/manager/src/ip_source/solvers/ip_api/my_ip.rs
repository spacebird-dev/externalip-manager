use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::time::Instant;
use tracing::{debug, instrument, warn};

use crate::ip_source::{AddressKind, SourceError, solvers::ip_api::IpProvider};

const MY_IP_URL_V4: &str = "https://api4.my-ip.io/v2/ip.json";
const MY_IP_URL_V6: &str = "https://api6.my-ip.io/v2/ip.json";

#[derive(Deserialize, Debug, Clone, Copy)]
enum Response {
    Success(MyIpResponse),
    RateLimited,
}

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

// cache responses to avoid getting rate limited
const CACHE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct MyIp {
    cached_response: Option<(Instant, Response)>,
}
impl MyIp {
    pub fn new() -> MyIp {
        MyIp {
            cached_response: None,
        }
    }
}
#[async_trait]
impl IpProvider for MyIp {
    #[instrument(skip(self, client))]
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        client: &Client,
    ) -> Result<Vec<std::net::IpAddr>, SourceError> {
        if let Some((instant, response)) = &self.cached_response
            && instant.elapsed() <= CACHE_TIMEOUT
        {
            match response {
                Response::Success(response) => {
                    debug!(msg = "Using cached MyIP response", ?response);
                    return Ok(vec![response.ip]);
                }
                Response::RateLimited => {
                    warn!(msg = "Rate-limited by MyIP, backing off and trying again in the future");
                    return Err(SourceError {
                        msg: "Rate-limited by MyIP".to_string(),
                    });
                }
            }
        }
        let now = Instant::now();
        let res = client
            .get(match kind {
                AddressKind::IPv4 => MY_IP_URL_V4,
                AddressKind::IPv6 => MY_IP_URL_V6,
            })
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            self.cached_response = Some((now, Response::RateLimited));
            return Err(SourceError {
                msg: "Rate-limited by MyIP".to_string(),
            });
        }
        let res = res.error_for_status()?.json::<MyIpResponse>().await?;
        self.cached_response = Some((Instant::now(), Response::Success(res)));
        Ok(vec![res.ip])
    }
}

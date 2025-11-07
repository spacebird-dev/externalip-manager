use std::net::IpAddr;
use std::{fmt::Debug, time::Duration};

use async_trait::async_trait;
use provider_myip::MyIp;
use reqwest::Client;
use tokio::time::Instant;

use crate::external_ip_source::solvers::SolverError;

use super::{AddressKind, Solver};
pub use solver::IpApiSolver;

mod provider_ipify;
mod provider_myip;
mod solver;

use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum IpProviderError {
    #[error("rate limited by IP provider, backing off for {secs} seconds", secs = remaining.as_secs())]
    RateLimited { remaining: Duration },
    #[error("IP provider request failed: `{0}`")]
    RequestFailed(String),
    #[error("IP provider response is invalid: `{0}`")]
    InvalidResponse(String),
}
impl From<IpProviderError> for SolverError {
    fn from(value: IpProviderError) -> Self {
        SolverError {
            reason: value.to_string(),
        }
    }
}
impl From<&IpProviderError> for SolverError {
    fn from(value: &IpProviderError) -> Self {
        SolverError::from((*value).clone())
    }
}

impl From<reqwest::Error> for IpProviderError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_decode() {
            IpProviderError::InvalidResponse(value.to_string())
        } else {
            IpProviderError::RequestFailed(value.to_string())
        }
    }
}

#[derive(Debug, Clone)]
struct IpProviderResponse {
    timeout: Duration,
    timestamp: Instant,
    expires_at: Instant,
    response: Result<Vec<IpAddr>, IpProviderError>,
}
impl IpProviderResponse {
    fn new(
        timeout: Duration,
        response: Result<Vec<IpAddr>, IpProviderError>,
    ) -> IpProviderResponse {
        let now = Instant::now();
        IpProviderResponse {
            timeout,
            timestamp: now,
            expires_at: now + timeout,
            response,
        }
    }
    fn expired(&self) -> bool {
        self.expires_at < Instant::now()
    }
    fn remaining(&self) -> Duration {
        self.timeout.saturating_sub(self.timestamp.elapsed())
    }
}

#[async_trait]
trait IpProvider: Send + Sync + Debug {
    async fn get_addresses(&mut self, kind: AddressKind, client: &Client) -> IpProviderResponse;
}

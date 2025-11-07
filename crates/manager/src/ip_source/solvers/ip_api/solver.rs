use std::{fmt::Debug, net::IpAddr, time::Duration};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use reqwest::Client;
use tokio::time::Instant;
use tracing::{debug, instrument};

use crate::{
    crd::v1alpha1,
    ip_source::{self, solvers::ip_api::ipify::Ipify},
};

use super::{IpProvider, IpSolverError, MyIp, Source, SourceError};

#[cfg(not(test))]
const CACHE_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(not(test))]
const RATELIMIT_BACKOFF_DURATION_BASE: Duration = Duration::from_secs(60 * 5);
const RATELIMIT_BACKOFF_DURATION_MAX: Duration = Duration::from_secs(60 * 60);

#[cfg(test)]
const CACHE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const RATELIMIT_BACKOFF_DURATION_BASE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct CachedResponse {
    timestamp: Instant,
    timeout: Duration,
    content: ResponseContent,
}
impl CachedResponse {
    fn new(content: ResponseContent, timeout: Duration) -> CachedResponse {
        CachedResponse {
            timestamp: Instant::now(),
            timeout,
            content,
        }
    }
    fn remaining(&self) -> Duration {
        self.timeout.saturating_sub(Instant::now() - self.timestamp)
    }
}
#[derive(Debug, Clone)]
enum ResponseContent {
    RateLimited,
    Addresses(Vec<IpAddr>),
}

#[derive(Debug)]
pub struct IpSolver {
    client: Client,
    inner: Box<dyn IpProvider>,
    cache: Option<CachedResponse>,
}

impl IpSolver {
    pub fn new(provider: v1alpha1::IpSolverProvider) -> IpSolver {
        let inner: Box<dyn IpProvider> = match provider {
            v1alpha1::IpSolverProvider::MyIp => Box::new(MyIp::new()),
            v1alpha1::IpSolverProvider::Ipify => Box::new(Ipify::new()),
        };
        IpSolver {
            client: Client::new(),
            inner,
            cache: None,
        }
    }
    #[cfg(test)]
    fn with_ip_provider(inner: Box<dyn IpProvider>) -> IpSolver {
        IpSolver {
            client: Client::new(),
            inner,
            cache: None,
        }
    }
}

#[async_trait]
impl Source for IpSolver {
    #[instrument(skip(self))]
    async fn get_addresses(
        &mut self,
        kind: ip_source::AddressKind,
        _: &Service,
    ) -> Result<Vec<std::net::IpAddr>, ip_source::SourceError> {
        if let Some(cached) = &self.cache
            && cached.remaining() > Duration::ZERO
        {
            return match &cached.content {
                ResponseContent::RateLimited => Err(SourceError {
                    msg: format!(
                        "Still backing off from rate limited IP API, {} seconds remaining...",
                        cached.remaining().as_secs()
                    ),
                }),
                ResponseContent::Addresses(addrs) => {
                    debug!(msg = "Reusing cached addresses for IP API", addresses = ?addrs.clone());
                    Ok(addrs.clone())
                }
            };
        }

        match self.inner.get_addresses(kind, &self.client).await {
            Ok(addrs) => {
                debug!(msg = "Resolved address through IP API", addresses = ?addrs.clone());
                self.cache = Some(CachedResponse {
                    timestamp: Instant::now(),
                    timeout: CACHE_TIMEOUT,
                    content: ResponseContent::Addresses(addrs.clone()),
                });
                Ok(addrs)
            }
            Err(IpSolverError::RateLimited) => {
                let new_cache = if let Some(cached) = &self.cache
                    && matches!(&cached.content, ResponseContent::RateLimited)
                {
                    CachedResponse::new(
                        ResponseContent::RateLimited,
                        // Exponential backoff
                        (cached.timeout * 2).min(RATELIMIT_BACKOFF_DURATION_MAX),
                    )
                } else {
                    CachedResponse::new(
                        ResponseContent::RateLimited,
                        RATELIMIT_BACKOFF_DURATION_BASE,
                    )
                };
                let backoff = new_cache.timeout.as_secs();
                self.cache = Some(new_cache);
                Err(SourceError {
                    msg: format!(
                        "Rate limited by IP API, backing off for {} seconds...",
                        backoff
                    ),
                })
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::ip_source::AddressKind;

    use super::*;

    #[derive(Debug)]
    struct MockSolver {
        count: usize,
        responses: Vec<Result<Vec<IpAddr>, IpSolverError>>,
    }
    impl MockSolver {
        fn new(responses: Vec<Result<Vec<IpAddr>, IpSolverError>>) -> MockSolver {
            MockSolver {
                count: 0,
                responses,
            }
        }
    }
    #[async_trait]
    impl IpProvider for MockSolver {
        async fn get_addresses(
            &mut self,
            _: AddressKind,
            _: &Client,
        ) -> Result<Vec<IpAddr>, IpSolverError> {
            let res = self.responses[self.count].clone();
            self.count += 1;
            res
        }
    }

    #[tokio::test]
    async fn uses_cache() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpSolver::with_ip_provider(Box::new(MockSolver::new(vec![
            Ok(expected.clone()),
            Ok(vec!["1.1.1.1".parse().unwrap()]),
        ])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await?;
        assert_eq!(result, expected);
        // Second call should reuse cached address
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn cache_invalidates() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpSolver::with_ip_provider(Box::new(MockSolver::new(vec![
            Ok(vec!["1.1.1.1".parse().unwrap()]),
            Ok(expected.clone()),
        ])));
        // first call to fill cache
        solv.get_addresses(AddressKind::IPv4, &Service::default())
            .await?;
        tokio::time::sleep(CACHE_TIMEOUT + Duration::from_secs(1)).await;
        // Call after sleep should be second address
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn errors_on_ratelimit() -> Result<()> {
        let mut solv = IpSolver::with_ip_provider(Box::new(MockSolver::new(vec![Err(
            IpSolverError::RateLimited,
        )])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn waits_after_ratelimit() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpSolver::with_ip_provider(Box::new(MockSolver::new(vec![
            Err(IpSolverError::RateLimited),
            Ok(expected.clone()),
        ])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await;
        assert!(result.is_err());
        // Immediate second query, should still return a rate limit error
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await;
        assert!(result.is_err());
        tokio::time::sleep(RATELIMIT_BACKOFF_DURATION_BASE + Duration::from_secs(1)).await;
        // After waitlimit timeout, the request succeeds
        // Immediate second query, should still return a rate limit error
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn exponential_backoff_on_repeated_ratelimit() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpSolver::with_ip_provider(Box::new(MockSolver::new(vec![
            Err(IpSolverError::RateLimited),
            Err(IpSolverError::RateLimited),
            Ok(expected.clone()),
        ])));
        // Trigger ratelimit
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await;
        assert!(result.is_err());
        tokio::time::sleep(RATELIMIT_BACKOFF_DURATION_BASE + Duration::from_secs(1)).await;
        // Still ratelimited
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default())
            .await;
        assert!(result.is_err());
        // Ratelimited with exponential backoff
        assert!(
            solv.cache.expect("should have cached ratelimit").timeout
                == RATELIMIT_BACKOFF_DURATION_BASE * 2
        );
        Ok(())
    }
}

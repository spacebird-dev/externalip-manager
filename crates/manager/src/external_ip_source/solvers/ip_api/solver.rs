use std::{fmt::Debug, time::Duration};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use reqwest::Client;
use tracing::{debug, info, instrument};

use crate::{
    crd::v1alpha1,
    external_ip_source::{
        self,
        registry::SolverRegistry,
        solvers::{
            SolverError,
            ip_api::{IpProviderResponse, provider_ipify::Ipify},
        },
    },
};

use super::{IpProvider, IpProviderError, MyIp, Solver};

const RATELIMIT_BACKOFF_DURATION_MAX: Duration = Duration::from_secs(60 * 60 * 2);

#[derive(Debug)]
pub struct IpApiSolver {
    client: Client,
    inner: Box<dyn IpProvider>,
    cache: Option<IpProviderResponse>,
}

impl IpApiSolver {
    pub fn new(provider: v1alpha1::IpSolverProvider) -> IpApiSolver {
        let inner: Box<dyn IpProvider> = match provider {
            v1alpha1::IpSolverProvider::MyIp => Box::new(MyIp::new()),
            v1alpha1::IpSolverProvider::Ipify => Box::new(Ipify::new()),
        };
        IpApiSolver {
            client: Client::new(),
            inner,
            cache: None,
        }
    }
    #[cfg(test)]
    fn with_test_provider(inner: Box<dyn IpProvider>) -> IpApiSolver {
        IpApiSolver {
            client: Client::new(),
            inner,
            cache: None,
        }
    }
}

#[async_trait]
impl Solver for IpApiSolver {
    #[instrument(skip(self))]
    async fn get_addresses(
        &mut self,
        kind: external_ip_source::AddressKind,
        _: &Service,
        _: &SolverRegistry,
    ) -> Result<Vec<std::net::IpAddr>, SolverError> {
        if let Some(cached) = &self.cache
            && !cached.expired()
        {
            match &cached.response {
                Ok(addrs) => {
                    info!(
                        msg = "reusing cached addresses for IP API",
                        cache_remaining_secs = cached.remaining().as_secs()
                    );
                    return Ok(addrs.clone());
                }
                Err(e) => {
                    if matches!(e, IpProviderError::RateLimited) {
                        debug!(msg = "respecting cached ratelimit response", resp = ?e);
                        return Err(e.into());
                    }
                }
            };
        }

        let resp = self.inner.get_addresses(kind, &self.client).await;
        let (res, cache) = match &resp.response {
            Ok(addrs) => (Ok(addrs.clone()), Some(resp.clone())),
            Err(e) if matches!(e, IpProviderError::RateLimited) => {
                if let Some(cached) = &self.cache
                    && matches!(&cached.response, Err(IpProviderError::RateLimited))
                {
                    // Exponential backoff
                    let new_timeout = (cached.timeout * 2).min(RATELIMIT_BACKOFF_DURATION_MAX);
                    debug!(
                        msg = "hit rate limit repeatedly, backing off exponentially",
                        backoff_secs = new_timeout.as_secs()
                    );
                    let mut exp_cache =
                        IpProviderResponse::new(new_timeout, cached.response.clone());
                    exp_cache.timeout = new_timeout;
                    (Err(IpProviderError::RateLimited.into()), Some(exp_cache))
                } else {
                    (Err(e.into()), Some(resp.clone()))
                }
            }
            Err(e) => (Err(e.into()), None),
        };
        self.cache = cache;
        res
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anyhow::Result;

    use crate::external_ip_source::AddressKind;

    use super::*;

    const CACHE_TIMEOUT: Duration = Duration::from_millis(500);
    const SLEEP_EXTRA: Duration = Duration::from_millis(100);

    #[derive(Debug)]
    struct MockSolver {
        count: usize,
        responses: Vec<IpProviderResponse>,
    }
    impl MockSolver {
        fn new(responses: Vec<IpProviderResponse>) -> MockSolver {
            MockSolver {
                count: 0,
                responses,
            }
        }
    }
    #[async_trait]
    impl IpProvider for MockSolver {
        async fn get_addresses(&mut self, _: AddressKind, _: &Client) -> IpProviderResponse {
            let res = self.responses[self.count].clone();
            self.count += 1;
            res
        }
    }

    #[tokio::test]
    async fn uses_cache() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpApiSolver::with_test_provider(Box::new(MockSolver::new(vec![
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(expected.clone())),
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(vec!["1.1.1.1".parse().unwrap()])),
        ])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await?;
        assert_eq!(result, expected);
        // Second call should reuse cached address
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn cache_invalidates() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpApiSolver::with_test_provider(Box::new(MockSolver::new(vec![
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(vec!["1.1.1.1".parse().unwrap()])),
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(expected.clone())),
        ])));
        // first call to fill cache
        solv.get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await?;
        tokio::time::sleep(CACHE_TIMEOUT + SLEEP_EXTRA).await;
        // Call after sleep should be second address
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn errors_on_ratelimit() -> Result<()> {
        let mut solv = IpApiSolver::with_test_provider(Box::new(MockSolver::new(vec![
            IpProviderResponse::new(CACHE_TIMEOUT, Err(IpProviderError::RateLimited)),
        ])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn waits_after_ratelimit() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpApiSolver::with_test_provider(Box::new(MockSolver::new(vec![
            IpProviderResponse::new(CACHE_TIMEOUT, Err(IpProviderError::RateLimited)),
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(expected.clone())),
        ])));
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await;
        assert!(result.is_err());
        // Immediate second query, should still return a rate limit error
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await;
        assert!(result.is_err());
        tokio::time::sleep(CACHE_TIMEOUT + SLEEP_EXTRA).await;
        // After waitlimit timeout, the request succeeds
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await?;
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn exponential_backoff_on_repeated_ratelimit() -> Result<()> {
        let expected = vec!["0.0.0.0".parse().unwrap()];
        let mut solv = IpApiSolver::with_test_provider(Box::new(MockSolver::new(vec![
            IpProviderResponse::new(CACHE_TIMEOUT, Err(IpProviderError::RateLimited)),
            IpProviderResponse::new(CACHE_TIMEOUT, Err(IpProviderError::RateLimited)),
            IpProviderResponse::new(CACHE_TIMEOUT, Ok(expected.clone())),
        ])));
        // Trigger ratelimit
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await;
        assert!(result.is_err());
        tokio::time::sleep(CACHE_TIMEOUT + SLEEP_EXTRA).await;
        // Still ratelimited
        let result = solv
            .get_addresses(AddressKind::IPv4, &Service::default(), &HashMap::default())
            .await;
        assert!(result.is_err());
        // Ratelimited with exponential backoff
        assert!(solv.cache.expect("should have cached ratelimit").timeout == CACHE_TIMEOUT * 2);
        Ok(())
    }
}

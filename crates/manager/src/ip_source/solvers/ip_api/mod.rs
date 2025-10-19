use std::{fmt::Debug, net::IpAddr};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use my_ip::MyIp;
use reqwest::Client;
use tracing::{debug, instrument};

use crate::ip_source;

use super::{AddressKind, Source, SourceError};

mod my_ip;

#[async_trait]
trait IpProvider: Send + Sync + Debug {
    async fn get_addresses(
        &mut self,
        kind: AddressKind,
        client: &Client,
    ) -> Result<Vec<IpAddr>, SourceError>;
}

#[derive(Debug)]
pub struct IpSolver {
    client: Client,
    inner: Box<dyn IpProvider>,
}

impl IpSolver {
    pub fn new(provider: crate::crd::v1alpha1::IpSolverProvider) -> IpSolver {
        let inner: Box<dyn IpProvider> = match provider {
            crate::crd::v1alpha1::IpSolverProvider::MyIp => Box::new(MyIp::new()),
        };
        IpSolver {
            client: Client::new(),
            inner,
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
        let res = self.inner.get_addresses(kind, &self.client).await;
        if let Ok(addrs) = &res {
            debug!(msg = "Resolved address through Ip API", addresses = ?addrs);
        }
        res
    }
}

impl From<reqwest::Error> for SourceError {
    fn from(value: reqwest::Error) -> Self {
        SourceError {
            msg: value.to_string(),
        }
    }
}

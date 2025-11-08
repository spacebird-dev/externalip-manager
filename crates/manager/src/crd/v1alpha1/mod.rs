use std::net::IpAddr;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CLUSTER_EXTERNAL_IP_SOURCE_KIND: &str = "ClusterExternalIPSource";

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "externalip.spacebird.dev",
    version = "v1alpha1",
    kind = "ClusterExternalIPSource",
    plural = "clusterexternalipsources",
    doc = "Cluster-Wide source of external IP addresses for a given service",
    category = "externalip-manager",
    shortname = "ceips"
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterExternalIpSourceSpec {
    /// Configure solvers for Ipv4 addresses
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv4: Option<IpSolversConfig>,
    /// Configure solvers for Ipv6 addresses
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<IpSolversConfig>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct IpSolversConfig {
    /// How the list of solvers should be queried. Can be "firstFound" (default) or "all".
    /// "firstFound" will query solvers until one succeeds and return only the addresses from this query.
    /// "all" will query all solvers and return all found addresses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_mode: Option<QueryMode>,
    #[serde(default)]
    pub solvers: Vec<SolverKind>,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum QueryMode {
    #[default]
    FirstFound,
    All,
}

#[derive(Deserialize, Serialize, Clone, Debug, Hash, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SolverKind {
    /// Use a public "What-is-my-ip"-style service to deduce external IP addresses
    #[serde(rename = "ipAPI")]
    IpAPI(IpAPIConfig),
    /// Resolve a hostname through DNS and return the resulting A/AAAA records as IP addresses
    DnsHostname(DnsHostnameConfig),
    /// Use the ingress addresses assigned to the service in .status.loadBalancer.ingress as external IP addresses
    LoadBalancerIngress(LoadBalancerIngressConfig),
    /// Return one or more static IP addresses. Useful as a fallback or as a partial address for the "merge" solver
    Static(StaticConfig),
    /// Merge the results from multiple solvers into a single address through masks. Useful for overriding a prefix or subnet from an acquired IP, or for merging a public prefix with a private address
    Merge(MergeConfig),
}
impl From<PartialSolverKind> for SolverKind {
    fn from(value: PartialSolverKind) -> Self {
        match value {
            PartialSolverKind::IpAPI(c) => SolverKind::IpAPI(c),
            PartialSolverKind::DnsHostname(c) => SolverKind::DnsHostname(c),
            PartialSolverKind::LoadBalancerIngress(c) => SolverKind::LoadBalancerIngress(c),
            PartialSolverKind::Static(c) => SolverKind::Static(c),
        }
    }
}
impl From<&PartialSolverKind> for SolverKind {
    fn from(value: &PartialSolverKind) -> Self {
        SolverKind::from((*value).clone())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsHostnameConfig {
    /// The host to resolve.
    pub host: String,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IpAPIConfig {
    /// The service to use for retrieving public IP information
    #[serde(default)]
    pub provider: IpSolverProvider,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IpSolverProvider {
    /// my-ip.io
    #[default]
    MyIp,
    // https://www.ipify.org/
    Ipify,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerIngressConfig {}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StaticConfig {
    /// Addresses to return. Addresses with mismatched types (v4 vs v6) will be ignored
    pub addresses: Vec<IpAddr>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeConfig {
    /// Each partial solver returns a section of the final IP address.
    /// Should a solver return multiple IP addresses, the last address is used
    pub partial_solvers: Vec<PartialSolver>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PartialSolver {
    /// Type of solver to retrieve the address part through.
    /// Should a solver return multiple IP addresses, the last address is used as the part
    pub solver: PartialSolverKind,
    /// This netmask defines the section of the solvers response that will be used in the final address. Examples: 0:0:0:ffff::, 0.0.255.0
    pub mask: IpAddr,
}

// TODO: Generate this and SolverKind through a macro as to avoid duplication
#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PartialSolverKind {
    /// Use a public "What-is-my-ip"-style service to deduce external IP addresses
    #[serde(rename = "ipAPI")]
    IpAPI(IpAPIConfig),
    /// Resolve a hostname through DNS and return the resulting A/AAAA records as IP addresses
    DnsHostname(DnsHostnameConfig),
    /// Use the ingress addresses assigned to the service in .status.loadBalancer.ingress as external IP addresses
    LoadBalancerIngress(LoadBalancerIngressConfig),
    /// Return one or more static IP addresses. Useful as a fallback or as a partial address for the "merge" solver
    Static(StaticConfig),
}

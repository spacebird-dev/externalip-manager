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
    doc = "Cluster-Wide source of external IP addresses for a given service"
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterExternalIpSourceSpec {
    /// Configure sources for Ipv4 addresses
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv4: Option<IpSourcesConfig>,
    /// Configure sources for Ipv6 addresses
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<IpSourcesConfig>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct IpSourcesConfig {
    /// How the list of sources should be queried. Can be "firstFound" (default) or "all".
    /// "firstFound" will query sources until one succeeds and return only the addresses from this query.
    /// "all" will query all sources and return all found addresses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_mode: Option<QueryMode>,
    #[serde(default)]
    pub sources: Vec<SourceKind>,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum QueryMode {
    #[default]
    FirstFound,
    All,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// Use a public "What-is-my-ip"-style service to deduce external IP addresses
    #[serde(rename = "ipSolver")]
    IPSolver(IpSolverConfig),
    /// Resolve a hostname through DNS and return the resulting A/AAAA records as IP addresses
    DnsHostname(DnsHostnameConfig),
    /// Use the ingress addresses assigned to the service in .status.loadBalancer.ingress as external IP addresses
    LoadBalancerIngress(LoadBalancerIngressConfig),
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DnsHostnameConfig {
    /// The host to resolve.
    pub host: String,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IpSolverConfig {
    /// The service to use for retrieving public IP information
    pub provider: IpSolverProvider,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum IpSolverProvider {
    /// my-ip.io
    MyIp,
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerIngressConfig {}

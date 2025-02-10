# externalip-manager

A Kubernetes operator to manage the `externalIP` field on Kubernetes services.

## Background

Today, many k8s clusters are deployed in private networks behind NATs, with only Ingress services exposed through port forwards.
This poses a problem for applications that need to know the external IP address of such a service, such as [`external-dns`](https://github.com/kubernetes-sigs/external-dns).
Within a private cluster, only the private network IPs will be available, requiring the use of workarounds to obtain the true publicly reachable IP address.

Kubernetes does have a built-in solution for this - the [`externalIP` field](https://kubernetes.io/docs/concepts/services-networking/service/#external-ips) present on all services:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: my-service
spec:
  selector:
    app.kubernetes.io/name: MyApp
  ports:
    - port: 80
  externalIPs:
    - 198.51.100.32
```

This field can inform applications about the true public/external IP of the service (for example, external-dns uses these IPs for its DNS records).
However, there is a catch, as mentioned in the [documentation](https://kubernetes.io/docs/concepts/services-networking/service/#external-ips):

> Kubernetes does not manage allocation of externalIPs; these are the responsibility of the cluster administrator.

This is where `externalip-manager` comes in.

## Overview

The purpose of this operator is to automate the management of the `externalIP` field in situations where manual assignment is unfeasible (such as dynamic IP addresses).
To do so, it uses a new resource type `ClusterExternalIPSource` containing one or more solvers for determining the external IP addresses:

```yaml
apiVersion: externalip.spacebird.dev/v1alpha1
kind: ClusterExternalIPSource
metadta:
    name: public
spec:
  ipv4:
    solvers:
      - dnsHostname:
          host: "cluster-public-ip.example.com"
  ipv6:
    solvers:
      - loadBalancerIngress: {}
```

Services can then select their `externalIP` source through an annotation:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: my-service
  annotations:
    externalip.spacebird.dev/cluster-external-ip-source: public
spec:
  type: LoadBalancer
  selector:
    app.kubernetes.io/name: MyApp
  ports:
    - port: 80
```

`externalip-manager` will then pick up this service and query the solvers in the `ClusterExternalIPSource` until valid IP addresses are found.
It will then write them into the `externalIP` field of the service and regularly check the sources for any changes.

The following solvers are currently available:

- `dnsHostname`: Perform a DNS query and use the IPs returned in `A`/`AAAA` records.
  - Use case: You have a firewall/NAT gateway that sets a DNS record with the public IP.
  - Parameters:
    - `host`: The host to resolve
- `ìpAPI`: Uses a "what-is-my-ip" style API to retrieve public addresses
  - Parameters:
    - `provider`: Which API Provider to use. Currently, the only option is [`myIp`](https://my-ip.io)
- `loadBalancerIngress`: Use the addresses specified in the `.status.loadBalancer.ingress` field
  - Use case: You have MetalLB or a similar LoadBalancer providing you with some public addresses
  - Parameters: None

## Installation

To install this operator, use the Helm chart at [spacebird-dev/charts](https://github.com/spacebird-dev/charts/tree/main/charts/externalip-manager).

## Building

This operator is built in Rust, using standard `cargo` tooling.
You may want to install the `just` command runner to run the recipes in the [`Justfile`](./Justfile).
`cross` is used for cross-compilation.

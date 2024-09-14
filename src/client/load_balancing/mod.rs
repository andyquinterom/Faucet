mod ip_extractor;
pub mod ip_hash;
pub mod round_robin;
use super::worker::WorkerConfig;
use crate::client::Client;
use crate::error::FaucetResult;
use crate::leak;
use hyper::Request;
pub use ip_extractor::IpExtractor;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;

use self::ip_hash::IpHash;
use self::round_robin::RoundRobin;

trait LoadBalancingStrategy {
    async fn entry(&self, ip: IpAddr) -> Client;
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, Eq, PartialEq, serde::Deserialize)]
#[serde(rename = "snake_case")]
pub enum Strategy {
    RoundRobin,
    IpHash,
}

impl FromStr for Strategy {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "round_robin" => Ok(Self::RoundRobin),
            "ip_hash" => Ok(Self::IpHash),
            _ => Err("invalid strategy"),
        }
    }
}

#[derive(Copy, Clone)]
enum DynLoadBalancer {
    IpHash(&'static ip_hash::IpHash),
    RoundRobin(&'static round_robin::RoundRobin),
}

impl LoadBalancingStrategy for DynLoadBalancer {
    async fn entry(&self, ip: IpAddr) -> Client {
        match self {
            DynLoadBalancer::RoundRobin(rr) => rr.entry(ip).await,
            DynLoadBalancer::IpHash(ih) => ih.entry(ip).await,
        }
    }
}

pub(crate) struct LoadBalancer {
    strategy: DynLoadBalancer,
    extractor: IpExtractor,
}

impl LoadBalancer {
    pub fn new(
        strategy: Strategy,
        extractor: IpExtractor,
        workers: &[WorkerConfig],
    ) -> FaucetResult<Self> {
        let strategy: DynLoadBalancer = match strategy {
            Strategy::RoundRobin => DynLoadBalancer::RoundRobin(leak!(RoundRobin::new(workers)?)),
            Strategy::IpHash => DynLoadBalancer::IpHash(leak!(IpHash::new(workers)?)),
        };
        Ok(Self {
            strategy,
            extractor,
        })
    }
    pub async fn get_client(&self, ip: IpAddr) -> FaucetResult<Client> {
        Ok(self.strategy.entry(ip).await)
    }
    pub fn extract_ip<B>(
        &self,
        request: &Request<B>,
        socket: Option<IpAddr>,
    ) -> FaucetResult<IpAddr> {
        self.extractor.extract(request, socket)
    }
}

impl Clone for LoadBalancer {
    fn clone(&self) -> Self {
        Self {
            strategy: self.strategy,
            extractor: self.extractor,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(
            Strategy::from_str("round_robin").unwrap(),
            Strategy::RoundRobin
        );
        assert_eq!(Strategy::from_str("ip_hash").unwrap(), Strategy::IpHash);
        assert!(Strategy::from_str("invalid").is_err());
    }

    #[test]
    fn test_load_balancer_new_round_robin() {
        let configs = Vec::new();
        let _ = LoadBalancer::new(Strategy::RoundRobin, IpExtractor::XForwardedFor, &configs)
            .expect("failed to create load balancer");
    }

    #[test]
    fn test_load_balancer_new_ip_hash() {
        let configs = Vec::new();
        let _ = LoadBalancer::new(Strategy::IpHash, IpExtractor::XForwardedFor, &configs)
            .expect("failed to create load balancer");
    }

    #[test]
    fn test_load_balancer_extract_ip() {
        let configs = Vec::new();
        let load_balancer =
            LoadBalancer::new(Strategy::RoundRobin, IpExtractor::XForwardedFor, &configs)
                .expect("failed to create load balancer");
        let request = Request::builder()
            .header("x-forwarded-for", "192.168.0.1")
            .body(())
            .unwrap();
        let ip = load_balancer
            .extract_ip(&request, Some("127.0.0.1".parse().unwrap()))
            .expect("failed to extract ip");

        assert_eq!(ip, "192.168.0.1".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_load_balancer_get_client() {
        use crate::client::ExtractSocketAddr;
        let configs = [
            WorkerConfig::dummy("test", "127.0.0.1:9999", true),
            WorkerConfig::dummy("test", "127.0.0.1:9998", true),
        ];
        let load_balancer =
            LoadBalancer::new(Strategy::RoundRobin, IpExtractor::XForwardedFor, &configs)
                .expect("failed to create load balancer");
        let ip = "192.168.0.1".parse().unwrap();
        let client = load_balancer
            .get_client(ip)
            .await
            .expect("failed to get client");
        assert_eq!(client.socket_addr(), "127.0.0.1:9999".parse().unwrap());

        let client = load_balancer
            .get_client(ip)
            .await
            .expect("failed to get client");

        assert_eq!(client.socket_addr(), "127.0.0.1:9998".parse().unwrap());
    }

    #[test]
    fn test_clone_load_balancer() {
        let configs = Vec::new();
        let load_balancer =
            LoadBalancer::new(Strategy::RoundRobin, IpExtractor::XForwardedFor, &configs)
                .expect("failed to create load balancer");
        let _ = load_balancer.clone();
    }
}

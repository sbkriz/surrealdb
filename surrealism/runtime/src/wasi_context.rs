use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use ipnet::IpNet;
use surrealism_types::err::PrefixErr;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder};

/// A resolved allow-list entry for WASI socket address filtering.
///
/// Mirrors the parsing and matching logic of SurrealDB's `NetTarget`
/// (`surrealdb/core/src/dbs/capabilities.rs`), adapted for checking against
/// `SocketAddr` values in the `socket_addr_check` callback.
#[derive(Debug, Clone)]
enum NetFilter {
	/// IP network (single IP as /32 or /128, or CIDR block). Matches any port.
	Net(IpNet),
	/// Specific IP:port pair. Parsed when `socket_addr` syntax is used (e.g.
	/// `192.168.1.1:80` or a hostname that resolves to IPs with a specific port).
	IpPort(IpAddr, u16),
}

impl NetFilter {
	fn matches(&self, addr: &SocketAddr) -> bool {
		match self {
			Self::Net(net) => net.contains(&addr.ip()),
			Self::IpPort(ip, port) => addr.ip() == *ip && addr.port() == *port,
		}
	}
}

/// Parse allow_net entries into resolved filters, mirroring `NetTarget::from_str`.
///
/// Parsing order (matches SurrealDB):
/// 1. Try `IpNet` (CIDR: `10.0.0.0/8`, `fd00::/64`)
/// 2. Try `IpAddr` (single IP: `192.168.1.1`, `::1`) -> stored as /32 or /128
/// 3. Try URL-based parsing (`http://{entry}`) to extract host + optional port, then resolve
///    hostnames to IPs via DNS
fn parse_filters(allow_net: &[String]) -> Vec<NetFilter> {
	let mut filters = Vec::new();
	for entry in allow_net {
		if let Ok(net) = entry.parse::<IpNet>() {
			filters.push(NetFilter::Net(net));
		} else if let Ok(ip) = entry.parse::<IpAddr>() {
			filters.push(NetFilter::Net(IpNet::from(ip)));
		} else if let Ok(url) = url::Url::parse(&format!("http://{entry}")) {
			let Some(host) = url.host() else {
				tracing::warn!(entry, "allow_net entry has no host after URL parse");
				continue;
			};

			// Url::parse normalises port 80 to None for http://, so recover
			// the original port from the raw entry string.
			let port: Option<u16> = entry.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok());

			let ip: IpAddr = match host {
				url::Host::Ipv4(ip) => ip.into(),
				url::Host::Ipv6(ip) => ip.into(),
				url::Host::Domain(domain) => {
					resolve_hostname(domain, port, &mut filters);
					continue;
				}
			};
			if let Some(port) = port {
				filters.push(NetFilter::IpPort(ip, port));
			} else {
				filters.push(NetFilter::Net(IpNet::from(ip)));
			}
		} else {
			tracing::warn!(entry, "failed to parse allow_net entry");
		}
	}
	filters
}

/// Resolve a hostname to IP addresses and add corresponding filters.
/// Uses blocking DNS resolution (acceptable at startup / module load time).
fn resolve_hostname(hostname: &str, port: Option<u16>, filters: &mut Vec<NetFilter>) {
	match (hostname, port.unwrap_or(80)).to_socket_addrs() {
		Ok(addrs) => {
			for addr in addrs {
				if let Some(port) = port {
					filters.push(NetFilter::IpPort(addr.ip(), port));
				} else {
					filters.push(NetFilter::Net(IpNet::from(addr.ip())));
				}
			}
		}
		Err(e) => {
			tracing::warn!(hostname, %e, "failed to resolve allow_net hostname");
		}
	}
}

pub fn build(fs_root: Option<&Path>, allow_net: &[String]) -> Result<(WasiCtx, ResourceTable)> {
	let mut builder = WasiCtxBuilder::new();
	builder.inherit_stdout().inherit_stderr();

	if allow_net.is_empty() {
		builder.allow_tcp(false);
		builder.allow_udp(false);
		builder.allow_ip_name_lookup(false);
	} else {
		let filters = Arc::new(parse_filters(allow_net));
		builder.socket_addr_check(move |addr, _use| {
			let allowed = filters.iter().any(|f| f.matches(&addr));
			Box::pin(async move { allowed })
		});
	}

	if let Some(root) = fs_root {
		builder
			.preopened_dir(root, "/", DirPerms::READ, FilePerms::READ)
			.prefix_err(|| "Failed to preopen filesystem directory")?;
	}
	Ok((builder.build(), ResourceTable::new()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_ip_address() {
		let filters = parse_filters(&["192.168.1.1".into()]);
		assert_eq!(filters.len(), 1);
		let addr: SocketAddr = "192.168.1.1:8080".parse().unwrap();
		assert!(filters[0].matches(&addr));
		let other: SocketAddr = "10.0.0.1:80".parse().unwrap();
		assert!(!filters[0].matches(&other));
	}

	#[test]
	fn parse_cidr() {
		let filters = parse_filters(&["10.0.0.0/8".into()]);
		assert_eq!(filters.len(), 1);
		let inside: SocketAddr = "10.1.2.3:443".parse().unwrap();
		assert!(filters[0].matches(&inside));
		let outside: SocketAddr = "192.168.1.1:443".parse().unwrap();
		assert!(!filters[0].matches(&outside));
	}

	#[test]
	fn parse_ip_with_port() {
		let filters = parse_filters(&["192.168.1.1:80".into()]);
		assert_eq!(filters.len(), 1);
		let exact: SocketAddr = "192.168.1.1:80".parse().unwrap();
		assert!(filters[0].matches(&exact));
		let wrong_port: SocketAddr = "192.168.1.1:443".parse().unwrap();
		assert!(!filters[0].matches(&wrong_port));
	}
}

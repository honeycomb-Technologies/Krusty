//! Local port discovery utilities.
//!
//! This module provides a cross-platform view of TCP listeners so higher
//! layers can build features like preview/port-forwarding without shelling out
//! to platform-specific tools.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use anyhow::Result;
use netstat2::{get_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState};

/// A discovered local TCP listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpListenerInfo {
    /// Listening port.
    pub port: u16,
    /// Local addresses observed for this port (v4/v6, wildcard, loopback).
    pub addresses: Vec<IpAddr>,
    /// Associated process ids if the platform can provide them.
    pub pids: Vec<u32>,
}

/// Discover currently listening TCP ports on the host.
pub fn discover_listening_tcp_ports() -> Result<Vec<TcpListenerInfo>> {
    let af_flags = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
    let proto_flags = ProtocolFlags::TCP;
    let sockets = get_sockets_info(af_flags, proto_flags)?;

    let mut listeners: BTreeMap<u16, ListenerAccumulator> = BTreeMap::new();
    for socket in sockets {
        let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };
        if tcp.state != TcpState::Listen {
            continue;
        }

        let entry = listeners.entry(tcp.local_port).or_default();
        entry.addresses.insert(tcp.local_addr);
        for pid in socket.associated_pids {
            entry.pids.insert(pid);
        }
    }

    Ok(listeners
        .into_iter()
        .map(|(port, acc)| TcpListenerInfo {
            port,
            addresses: acc.addresses.into_iter().collect(),
            pids: acc.pids.into_iter().collect(),
        })
        .collect())
}

#[derive(Default)]
struct ListenerAccumulator {
    addresses: BTreeSet<IpAddr>,
    pids: BTreeSet<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_info_equality_is_structural() {
        let a = TcpListenerInfo {
            port: 5173,
            addresses: vec![
                "127.0.0.1".parse::<IpAddr>().expect("parse ipv4 loopback"),
                "::1".parse::<IpAddr>().expect("parse ipv6 loopback"),
            ],
            pids: vec![1234],
        };
        let b = TcpListenerInfo {
            port: 5173,
            addresses: vec![
                "127.0.0.1".parse::<IpAddr>().expect("parse ipv4 loopback"),
                "::1".parse::<IpAddr>().expect("parse ipv6 loopback"),
            ],
            pids: vec![1234],
        };

        assert_eq!(a, b);
    }

    #[test]
    fn discovery_is_permission_safe() {
        let result = discover_listening_tcp_ports();
        if let Err(err) = result {
            let text = format!("{:#}", err).to_ascii_lowercase();
            let expected_permission_error = text.contains("operation not permitted")
                || text.contains("permission denied")
                || text.contains("failed to call ffi");
            assert!(
                expected_permission_error,
                "unexpected discovery failure: {}",
                text
            );
        }
    }
}

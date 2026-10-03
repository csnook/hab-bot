//! IP addresses of open connections, kept in memory only.
//!
//! They exist for rate limits and connection caps (later tickets). Nothing
//! here writes them anywhere, and the server logs one only at debug level.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Peers {
    open: Mutex<HashMap<IpAddr, usize>>,
}

/// Held while a connection is open.
pub struct Guard {
    peers: Arc<Peers>,
    ip: IpAddr,
}

impl Peers {
    pub fn enter(self: &Arc<Self>, ip: IpAddr) -> Guard {
        *self.open.lock().unwrap().entry(ip).or_default() += 1;
        Guard {
            peers: self.clone(),
            ip,
        }
    }

    pub fn connections_from(&self, ip: IpAddr) -> usize {
        self.open.lock().unwrap().get(&ip).copied().unwrap_or(0)
    }

    pub fn distinct(&self) -> usize {
        self.open.lock().unwrap().len()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut open = self.peers.open.lock().unwrap();
        if let Some(n) = open.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                open.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_open_connections_and_forgets_closed_ones() {
        let peers = Arc::new(Peers::default());
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let a = peers.enter(ip);
        let b = peers.enter(ip);
        assert_eq!(peers.connections_from(ip), 2);
        drop(a);
        assert_eq!(peers.connections_from(ip), 1);
        drop(b);
        assert_eq!(peers.connections_from(ip), 0);
        assert_eq!(peers.distinct(), 0);
    }
}

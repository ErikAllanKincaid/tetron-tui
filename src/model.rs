//! Owned, UI-facing snapshot of daemon status, plus per-peer history ring
//! buffers. The UI holds these rather than `tetron_proto` types directly so a
//! protocol change touches only the mapping in [`map_status`], not the widgets.

use std::collections::VecDeque;
use std::time::Instant;
use tetron_proto::ipc::{ConnType, IpcMessage, ViaDetail};

/// What the IPC client reports to the UI each cycle.
#[derive(Clone)]
pub enum ConnStatus {
    /// Daemon answered a `Status` request.
    Up(Snapshot),
    /// Daemon unreachable or returned an error; carries a reason to display.
    Down(String),
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub active: bool,
    pub daemon_version: String,
    pub networks: Vec<NetSnap>,
}

#[derive(Clone)]
pub struct NetSnap {
    pub name: String,
    pub role: String,
    pub my_ip: String,
    pub subnet: String,
    pub tun_name: String,
    pub member_count: usize,
    pub active: bool,
    pub peers: Vec<PeerSnap>,
}

impl NetSnap {
    /// Peers with a live connection right now.
    pub fn connected(&self) -> usize {
        self.peers.iter().filter(|p| p.conn.is_some()).count()
    }
}

#[derive(Clone)]
pub struct PeerSnap {
    /// Stable identity, used as the history-map key.
    pub key: String,
    pub hostname: String,
    pub ip: String,
    pub is_coordinator: bool,
    pub last_seen: Option<u64>,
    pub conn: Option<ConnSnap>,
}

#[derive(Clone)]
pub struct ConnSnap {
    pub conn_type: ConnType,
    pub remote_addr: Option<String>,
    pub rtt_ms: Option<f64>,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub lost_packets: u64,
    pub max_datagram_size: Option<u64>,
    pub via_detail: Option<ViaDetail>,
    pub paths: Vec<PathSnap>,
}

#[derive(Clone)]
pub struct PathSnap {
    pub conn_type: ConnType,
    pub remote_addr: String,
    pub is_selected: bool,
    pub in_subnet: bool,
    pub has_activity: bool,
    pub rtt_ms: Option<f64>,
    pub current_mtu: Option<u16>,
}

/// Map a daemon reply into a UI snapshot. Returns `None` for any message that
/// is not a `StatusResponse` (e.g. a stray `Ok`/`Error`), which the caller
/// simply ignores.
pub fn map_status(msg: IpcMessage) -> Option<Snapshot> {
    let IpcMessage::StatusResponse { active, daemon_version, networks, .. } = msg else {
        return None;
    };
    let networks = networks
        .into_iter()
        .map(|n| NetSnap {
            name: n.network,
            role: n.role.to_string(),
            my_ip: n.my_ip.to_string(),
            subnet: n.subnet,
            tun_name: n.tun_name,
            member_count: n.member_count,
            active: n.active,
            peers: n
                .peers
                .into_iter()
                .map(|p| PeerSnap {
                    key: p.endpoint_id.to_string(),
                    hostname: p.hostname.unwrap_or_else(|| "?".into()),
                    ip: p.ip.to_string(),
                    is_coordinator: p.is_coordinator,
                    last_seen: p.last_seen,
                    conn: p.connection.map(|c| ConnSnap {
                        conn_type: c.conn_type,
                        remote_addr: c.remote_addr,
                        rtt_ms: c.rtt_ms,
                        bytes_tx: c.bytes_tx,
                        bytes_rx: c.bytes_rx,
                        lost_packets: c.lost_packets,
                        max_datagram_size: c.max_datagram_size,
                        via_detail: c.via_detail,
                        paths: c
                            .paths
                            .into_iter()
                            .map(|p| PathSnap {
                                conn_type: p.conn_type,
                                remote_addr: p.remote_addr,
                                is_selected: p.is_selected,
                                in_subnet: p.in_subnet,
                                has_activity: p.has_activity,
                                rtt_ms: p.rtt_ms,
                                current_mtu: p.current_mtu,
                            })
                            .collect(),
                    }),
                })
                .collect(),
        })
        .collect();
    Some(Snapshot { active, daemon_version, networks })
}

/// Rolling per-peer series for the History tab. Rates are derived from the
/// byte-counter deltas between successive polls.
pub struct PeerHistory {
    pub rtt: VecDeque<u64>,
    pub tx_rate: VecDeque<u64>,
    pub rx_rate: VecDeque<u64>,
    cap: usize,
    last: Option<(Instant, u64, u64)>,
}

impl PeerHistory {
    pub fn new(cap: usize) -> Self {
        PeerHistory { rtt: VecDeque::new(), tx_rate: VecDeque::new(), rx_rate: VecDeque::new(), cap, last: None }
    }

    /// Append one sample. `rtt_ms` is stored as whole milliseconds; the rates
    /// are bytes/second computed from the delta since the previous sample.
    pub fn push(&mut self, now: Instant, bytes_tx: u64, bytes_rx: u64, rtt_ms: Option<f64>) {
        if let Some((t0, tx0, rx0)) = self.last {
            let secs = now.duration_since(t0).as_secs_f64().max(0.001);
            let tx = ((bytes_tx.saturating_sub(tx0)) as f64 / secs) as u64;
            let rx = ((bytes_rx.saturating_sub(rx0)) as f64 / secs) as u64;
            push_capped(&mut self.tx_rate, tx, self.cap);
            push_capped(&mut self.rx_rate, rx, self.cap);
        }
        push_capped(&mut self.rtt, rtt_ms.unwrap_or(0.0).round() as u64, self.cap);
        self.last = Some((now, bytes_tx, bytes_rx));
    }
}

fn push_capped(q: &mut VecDeque<u64>, v: u64, cap: usize) {
    q.push_back(v);
    while q.len() > cap {
        q.pop_front();
    }
}

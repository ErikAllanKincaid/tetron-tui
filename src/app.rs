//! UI state and input handling. Slice A is read-only: input only moves the
//! selection and switches tabs; nothing changes daemon state.

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseEventKind};
use std::collections::HashMap;
use std::time::Instant;

use crate::model::{ConnStatus, NetSnap, PeerHistory, PeerSnap, Snapshot};

/// How many samples each history series retains (seconds at the 1s poll).
const HISTORY_CAP: usize = 240;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Peers,
    Transport,
    History,
}

impl Tab {
    pub fn all() -> [Tab; 3] {
        [Tab::Peers, Tab::Transport, Tab::History]
    }
    pub fn title(self) -> &'static str {
        match self {
            Tab::Peers => "Peers",
            Tab::Transport => "Transport",
            Tab::History => "History",
        }
    }
}

pub struct App {
    pub should_quit: bool,
    pub snapshot: Option<Snapshot>,
    pub down_reason: Option<String>,
    pub sel_net: usize,
    pub sel_peer: usize,
    pub tab: Tab,
    pub show_help: bool,
    pub history: HashMap<String, PeerHistory>,
}

impl App {
    pub fn new() -> Self {
        App {
            should_quit: false,
            snapshot: None,
            down_reason: Some("connecting…".into()),
            sel_net: 0,
            sel_peer: 0,
            tab: Tab::Peers,
            show_help: false,
            history: HashMap::new(),
        }
    }

    /// Fold one client report into state.
    pub fn on_status(&mut self, status: ConnStatus) {
        match status {
            ConnStatus::Up(snap) => {
                let now = Instant::now();
                let mut seen = std::collections::HashSet::new();
                for net in &snap.networks {
                    for p in &net.peers {
                        if let Some(c) = &p.conn {
                            seen.insert(p.key.clone());
                            self.history
                                .entry(p.key.clone())
                                .or_insert_with(|| PeerHistory::new(HISTORY_CAP))
                                .push(now, c.bytes_tx, c.bytes_rx, c.rtt_ms);
                        }
                    }
                }
                // Forget peers that are no longer connected anywhere.
                self.history.retain(|k, _| seen.contains(k));
                self.snapshot = Some(snap);
                self.down_reason = None;
                self.clamp();
            }
            ConnStatus::Down(reason) => {
                self.snapshot = None;
                self.down_reason = Some(reason);
            }
        }
    }

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k.code),
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => self.move_peer(1),
                MouseEventKind::ScrollUp => self.move_peer(-1),
                _ => {}
            },
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        if self.show_help {
            // Any key closes help.
            self.show_help = false;
            return;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Char('1') => self.tab = Tab::Peers,
            KeyCode::Char('2') => self.tab = Tab::Transport,
            KeyCode::Char('3') => self.tab = Tab::History,
            KeyCode::Up | KeyCode::Char('k') => self.move_peer(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_peer(1),
            KeyCode::Left | KeyCode::Char('h') => self.move_net(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_net(1),
            _ => {}
        }
    }

    fn cycle_tab(&mut self, d: i32) {
        let tabs = Tab::all();
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0) as i32;
        let n = tabs.len() as i32;
        self.tab = tabs[(((i + d) % n + n) % n) as usize];
    }

    fn move_net(&mut self, d: i32) {
        let n = self.net_count();
        if n == 0 {
            return;
        }
        self.sel_net = wrap(self.sel_net, d, n);
        self.sel_peer = 0;
    }

    fn move_peer(&mut self, d: i32) {
        let n = self.peer_count();
        if n == 0 {
            return;
        }
        self.sel_peer = wrap(self.sel_peer, d, n);
    }

    fn clamp(&mut self) {
        let nets = self.net_count();
        if nets == 0 {
            self.sel_net = 0;
            self.sel_peer = 0;
            return;
        }
        if self.sel_net >= nets {
            self.sel_net = nets - 1;
        }
        let peers = self.peer_count();
        if peers == 0 {
            self.sel_peer = 0;
        } else if self.sel_peer >= peers {
            self.sel_peer = peers - 1;
        }
    }

    pub fn net_count(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |s| s.networks.len())
    }

    pub fn peer_count(&self) -> usize {
        self.cur_net().map_or(0, |n| n.peers.len())
    }

    pub fn cur_net(&self) -> Option<&NetSnap> {
        self.snapshot.as_ref().and_then(|s| s.networks.get(self.sel_net))
    }

    pub fn cur_peer(&self) -> Option<&PeerSnap> {
        self.cur_net().and_then(|n| n.peers.get(self.sel_peer))
    }
}

fn wrap(cur: usize, d: i32, n: usize) -> usize {
    let n = n as i32;
    (((cur as i32 + d) % n + n) % n) as usize
}

//! UI state and input handling.
//!
//! Slice A was read-only. Slice B adds mutating actions, dispatched to the
//! [`crate::action`] worker over a channel: Resume/Standby (per-network and
//! global), invite create (shown as text + QR), join by code, create network,
//! and daemon stop/start. Text entry uses a modal input with bracketed paste.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::{Action, ActionResult};
use crate::model::{ConnStatus, NetSnap, PeerHistory, PeerSnap, Snapshot};

/// Clickable regions recorded by the render pass, consumed by mouse handling
/// on the next event (layout is stable between a draw and the next input).
#[derive(Default)]
pub struct Hits {
    /// Sidebar network rows, indexed by network position.
    pub nets: Vec<Rect>,
    /// Tab labels and the tab each selects.
    pub tabs: Vec<(Rect, Tab)>,
    /// Peer table rows, indexed by peer position within the current network.
    pub peers: Vec<Rect>,
}

/// How many samples each history series retains (seconds at the 1s poll).
const HISTORY_CAP: usize = 240;
/// How long a toast stays on screen.
const TOAST_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Peers,
    Transport,
    History,
    Admin,
}

impl Tab {
    pub fn all() -> [Tab; 4] {
        [Tab::Peers, Tab::Transport, Tab::History, Tab::Admin]
    }
    pub fn title(self) -> &'static str {
        match self {
            Tab::Peers => "Peers",
            Tab::Transport => "Transport",
            Tab::History => "History",
            Tab::Admin => "Admin",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Join,
    Create,
}

/// A modal overlay. `Normal` means the main view is interactive.
pub enum Mode {
    Normal,
    Input { kind: InputKind, buf: String },
    Invite { network: String, code: String, id: String, qr: String },
    /// A yes/no gate before firing a (possibly destructive) action.
    Confirm { prompt: String, action: Action },
}

pub struct Toast {
    pub text: String,
    pub err: bool,
    at: Instant,
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
    pub mode: Mode,
    pub toast: Option<Toast>,
    /// Key-holders of a network (name, [(short_id, is_self)]), for the Admin
    /// tab. Fetched on demand via `AdminList`.
    pub admins: Option<(String, Vec<(String, bool)>)>,
    /// The network an AdminList is currently requested/cached for (dedupe).
    admins_req: Option<String>,
    /// Clickable regions from the last render.
    pub hits: Hits,
    action_tx: UnboundedSender<Action>,
}

impl App {
    pub fn new(action_tx: UnboundedSender<Action>) -> Self {
        App {
            should_quit: false,
            snapshot: None,
            down_reason: Some("connecting…".into()),
            sel_net: 0,
            sel_peer: 0,
            tab: Tab::Peers,
            show_help: false,
            history: HashMap::new(),
            mode: Mode::Normal,
            toast: None,
            admins: None,
            admins_req: None,
            hits: Hits::default(),
            action_tx,
        }
    }

    // --- inbound updates ---------------------------------------------------

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

    pub fn on_action_result(&mut self, r: ActionResult) {
        match r {
            ActionResult::Ok(m) => self.set_toast(m, false),
            ActionResult::Err(m) => self.set_toast(m, true),
            ActionResult::Invite { network, code, id } => {
                let qr = build_qr(&code);
                self.mode = Mode::Invite { network, code, id, qr };
            }
            ActionResult::Admins { network, admins } => {
                self.admins = Some((network, admins));
            }
        }
    }

    /// Drop an expired toast. Called from the render path each frame.
    pub fn tick_toast(&mut self) {
        if let Some(t) = &self.toast {
            if t.at.elapsed() > TOAST_TTL {
                self.toast = None;
            }
        }
    }

    // --- input -------------------------------------------------------------

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Paste(s) => {
                if let Mode::Input { buf, .. } = &mut self.mode {
                    buf.push_str(s.trim());
                }
            }
            Event::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.show_help {
                    self.show_help = false;
                    return;
                }
                match self.mode {
                    // A click dismisses the invite card; other modals need keys.
                    Mode::Invite { .. } => self.mode = Mode::Normal,
                    Mode::Normal => self.on_click(m.column, m.row),
                    _ => {}
                }
            }
            MouseEventKind::ScrollDown if self.interactive() => self.move_peer(1),
            MouseEventKind::ScrollUp if self.interactive() => self.move_peer(-1),
            _ => {}
        }
    }

    /// Resolve a left-click to a tab, network, or peer via the last render's
    /// hit map. Targets are collected before mutating to satisfy the borrow
    /// checker.
    fn on_click(&mut self, col: u16, row: u16) {
        let tab = self.hits.tabs.iter().find(|(r, _)| hit(r, col, row)).map(|(_, t)| *t);
        if let Some(t) = tab {
            self.set_tab(t);
            return;
        }
        let net = self.hits.nets.iter().position(|r| hit(r, col, row));
        if let Some(i) = net {
            if i < self.net_count() {
                self.sel_net = i;
                self.sel_peer = 0;
                self.maybe_fetch_admins();
            }
            return;
        }
        let peer = self.hits.peers.iter().position(|r| hit(r, col, row));
        if let Some(i) = peer {
            if i < self.peer_count() {
                self.sel_peer = i;
            }
        }
    }

    fn interactive(&self) -> bool {
        matches!(self.mode, Mode::Normal) && !self.show_help
    }

    fn on_key(&mut self, k: KeyEvent) {
        // Modal overlays consume input first.
        match self.mode {
            Mode::Input { .. } => return self.input_key(k),
            Mode::Invite { .. } => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.mode = Mode::Normal;
                }
                return;
            }
            Mode::Confirm { .. } => {
                let yes = matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                if let Mode::Confirm { action, .. } = std::mem::replace(&mut self.mode, Mode::Normal) {
                    if yes {
                        self.act(action);
                        self.set_toast("working…", false);
                    }
                }
                return;
            }
            Mode::Normal => {}
        }

        if self.show_help {
            self.show_help = false;
            return;
        }

        // Daemon-down context: only quit / help / start.
        if self.snapshot.is_none() {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Char('?') => self.show_help = true,
                KeyCode::Char('s') => {
                    self.act(Action::StartDaemon);
                    self.set_toast("starting daemon…", false);
                }
                _ => {}
            }
            return;
        }

        // Connected, normal navigation + actions.
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Char('1') => self.set_tab(Tab::Peers),
            KeyCode::Char('2') => self.set_tab(Tab::Transport),
            KeyCode::Char('3') => self.set_tab(Tab::History),
            KeyCode::Char('4') => self.set_tab(Tab::Admin),
            KeyCode::Up | KeyCode::Char('k') => self.move_peer(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_peer(1),
            KeyCode::Left | KeyCode::Char('h') => self.move_net(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_net(1),
            // Actions.
            KeyCode::Char('c') => self.set_active_sel(true),
            KeyCode::Char('d') => self.set_active_sel(false),
            KeyCode::Char('C') => {
                self.act(Action::SetActive { network: None, active: true });
                self.set_toast("resuming all…", false);
            }
            KeyCode::Char('D') => {
                self.act(Action::SetActive { network: None, active: false });
                self.set_toast("standby all…", false);
            }
            KeyCode::Char('i') => {
                if let Some(name) = self.cur_net().map(|n| n.name.clone()) {
                    self.act(Action::InviteCreate { network: name });
                    self.set_toast("requesting invite…", false);
                }
            }
            KeyCode::Char('J') => self.mode = Mode::Input { kind: InputKind::Join, buf: String::new() },
            KeyCode::Char('n') => self.mode = Mode::Input { kind: InputKind::Create, buf: String::new() },
            KeyCode::Char('S') => {
                self.mode = Mode::Confirm {
                    prompt: "Stop the tetron daemon? This severs this UI's control socket.".into(),
                    action: Action::Shutdown,
                }
            }
            // Admin (slice C).
            KeyCode::Char('a') => self.admin_add_sel(),
            KeyCode::Char('K') => self.kick_sel(),
            KeyCode::Char('L') => self.leave_sel(),
            KeyCode::Char('X') => self.nuke_sel(),
            _ => {}
        }
    }

    fn input_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                if let Mode::Input { kind, buf } = std::mem::replace(&mut self.mode, Mode::Normal) {
                    let v = buf.trim().to_string();
                    if v.is_empty() {
                        return;
                    }
                    match kind {
                        InputKind::Join => {
                            self.act(Action::Join { code: v });
                            self.set_toast("joining…", false);
                        }
                        InputKind::Create => {
                            self.act(Action::Create { name: v });
                            self.set_toast("creating…", false);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if let Mode::Input { buf, .. } = &mut self.mode {
                    buf.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Mode::Input { buf, .. } = &mut self.mode {
                    buf.push(c);
                }
            }
            _ => {}
        }
    }

    /// Resume/Standby the currently selected network.
    fn set_active_sel(&mut self, active: bool) {
        if let Some(name) = self.cur_net().map(|n| n.name.clone()) {
            self.act(Action::SetActive { network: Some(name.clone()), active });
            self.set_toast(
                format!("{} {name}", if active { "resuming" } else { "standby" }),
                false,
            );
        }
    }

    // --- admin actions (slice C) ------------------------------------------

    /// Grant the network key to the selected peer (admins only).
    fn admin_add_sel(&mut self) {
        let info = self
            .cur_net()
            .zip(self.cur_peer())
            .map(|(n, p)| (n.is_admin(), n.name.clone(), p.key.clone(), p.hostname.clone()));
        let Some((admin, network, peer, host)) = info else { return };
        if !admin {
            self.set_toast("admins only", true);
            return;
        }
        self.mode = Mode::Confirm {
            prompt: format!("Grant admin key to {host}? (makes it a co-coordinator)"),
            action: Action::AdminAdd { network, peer },
        };
    }

    /// Kick the selected peer from the network (admins only).
    fn kick_sel(&mut self) {
        let info = self.cur_net().zip(self.cur_peer()).and_then(|(n, p)| {
            n.network_key.clone().map(|k| (n.is_admin(), k, p.key.clone(), p.hostname.clone()))
        });
        let Some((admin, nk, pid, host)) = info else {
            self.set_toast("no network key", true);
            return;
        };
        if !admin {
            self.set_toast("admins only", true);
            return;
        }
        self.mode = Mode::Confirm {
            prompt: format!("Kick {host} from the network?"),
            action: Action::Kick { network_key: nk, endpoint_id: pid },
        };
    }

    /// Leave the selected network (any member).
    fn leave_sel(&mut self) {
        let Some(name) = self.cur_net().map(|n| n.name.clone()) else { return };
        self.mode = Mode::Confirm {
            prompt: format!("Leave network {name}?"),
            action: Action::Leave { network: name },
        };
    }

    /// Nuke the selected network, or second a pending proposal (admins only).
    fn nuke_sel(&mut self) {
        let info = self.cur_net().and_then(|n| {
            n.network_key.clone().map(|k| {
                (n.is_admin(), n.name.clone(), k, n.nuke_proposals.first().map(|(s, _)| s.clone()))
            })
        });
        let Some((admin, name, nk, pending)) = info else {
            self.set_toast("no network key", true);
            return;
        };
        if !admin {
            self.set_toast("admins only", true);
            return;
        }
        let (prompt, second) = match pending {
            Some(sid) => (
                format!("SECOND the pending nuke of {name} (by {sid})? This DESTROYS the network."),
                Some(sid),
            ),
            None => (
                format!("NUKE {name}? This permanently destroys the network for every member."),
                None,
            ),
        };
        self.mode = Mode::Confirm { prompt, action: Action::Nuke { network_key: nk, second } };
    }

    fn act(&self, a: Action) {
        let _ = self.action_tx.send(a);
    }

    /// Switch panes; fetch key-holders when entering the Admin tab.
    fn set_tab(&mut self, t: Tab) {
        self.tab = t;
        self.maybe_fetch_admins();
    }

    /// Request the current network's key-holders if the Admin tab is showing
    /// and we do not already have them for it.
    fn maybe_fetch_admins(&mut self) {
        if self.tab != Tab::Admin {
            return;
        }
        let Some(name) = self.cur_net().map(|n| n.name.clone()) else { return };
        if self.admins_req.as_deref() != Some(name.as_str()) {
            self.admins_req = Some(name.clone());
            self.admins = None;
            self.act(Action::AdminList { network: name });
        }
    }

    fn set_toast(&mut self, text: impl Into<String>, err: bool) {
        self.toast = Some(Toast { text: text.into(), err, at: Instant::now() });
    }

    // --- selection ---------------------------------------------------------

    fn cycle_tab(&mut self, d: i32) {
        let tabs = Tab::all();
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.set_tab(tabs[wrap(i, d, tabs.len())]);
    }

    fn move_net(&mut self, d: i32) {
        let n = self.net_count();
        if n == 0 {
            return;
        }
        self.sel_net = wrap(self.sel_net, d, n);
        self.sel_peer = 0;
        self.maybe_fetch_admins();
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

/// Point-in-rect test for a cell coordinate.
fn hit(r: &Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

/// Render an invite code as a unicode half-block QR. Falls back to a note if
/// the data is too large to encode.
fn build_qr(data: &str) -> String {
    use qrcode::render::unicode;
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => code.render::<unicode::Dense1x2>().quiet_zone(true).build(),
        Err(_) => "(code too large for QR)".into(),
    }
}

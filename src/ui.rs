//! Rendering. btop-style, truecolor. Layout: a status bar, a body split into a
//! network sidebar and a tabbed main area (Peers / Transport / History), and a
//! key-hint bar. An unreachable daemon shows a centered "daemon down" screen.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, Wrap};
use std::time::{SystemTime, UNIX_EPOCH};
use tetron_proto::ipc::{ConnType, ViaDetail};

use crate::app::{App, InputKind, Mode, Tab};
use crate::model::{NetSnap, PeerSnap};

const C_DIM: Color = Color::DarkGray;
const C_OK: Color = Color::Green;
const C_WARN: Color = Color::Yellow;
const C_BAD: Color = Color::Red;
const C_ACCENT: Color = Color::Cyan;

pub fn draw(f: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    draw_statusbar(f, rows[0], app);

    if app.snapshot.is_none() {
        draw_down(f, rows[1], app);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(1)])
            .split(rows[1]);
        draw_sidebar(f, body[0], app);
        draw_main(f, body[1], app);
    }

    draw_hintbar(f, rows[2], app);

    match &app.mode {
        Mode::Input { kind, buf } => draw_input(f, *kind, buf),
        Mode::Invite { network, code, id, qr } => draw_invite(f, network, code, id, qr),
        Mode::ConfirmStop => draw_confirm(f),
        Mode::Normal => {
            if app.show_help {
                draw_help(f);
            }
        }
    }
}

fn draw_statusbar(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(" tetron ", Style::new().fg(Color::Black).bg(C_ACCENT).bold())];
    if let Some(s) = &app.snapshot {
        let (word, col) = if s.active { ("active", C_OK) } else { ("standby", C_WARN) };
        let peers: usize = s.networks.iter().map(|n| n.connected()).sum();
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("⇄ {word}"), Style::new().fg(col)));
        spans.push(Span::styled(
            format!("  ·  {} net  ·  {} peers", s.networks.len(), peers),
            Style::new().fg(C_DIM),
        ));
        if !s.daemon_version.is_empty() {
            spans.push(Span::styled(format!("   daemon v{}", s.daemon_version), Style::new().fg(C_DIM)));
        }
    } else {
        spans.push(Span::styled("  daemon down", Style::new().fg(C_BAD)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_hintbar(f: &mut Frame, area: Rect, app: &App) {
    if let Some(t) = &app.toast {
        let col = if t.err { C_BAD } else { C_OK };
        let line = Line::from(vec![
            Span::styled(if t.err { " ✗ " } else { " ✓ " }, Style::new().fg(Color::Black).bg(col).bold()),
            Span::styled(format!(" {}", t.text), Style::new().fg(col)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    let hint = "q quit · ? help · Tab pane · ←→ net · ↑↓ peer · c/d conn · i invite · n new · J join · S stop";
    f.render_widget(Paragraph::new(Span::styled(hint, Style::new().fg(C_DIM))), area);
}

fn draw_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Networks ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(s) = &app.snapshot {
        for (i, net) in s.networks.iter().enumerate() {
            let sel = i == app.sel_net;
            let marker = if sel { "▸ " } else { "  " };
            let (dot, dcol) = if net.active { ("●", C_OK) } else { ("○", C_DIM) };
            let name_style = if sel {
                Style::new().fg(C_ACCENT).bold()
            } else {
                Style::new()
            };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::new().fg(C_ACCENT)),
                Span::styled(format!("{:<12}", trunc(&net.name, 12)), name_style),
                Span::styled(dot, Style::new().fg(dcol)),
                Span::styled(format!(" {}/{}", net.connected(), net.member_count), Style::new().fg(C_DIM)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("  c connect   d standby", Style::new().fg(C_DIM)));
    lines.push(Line::styled("  C/D all     i invite", Style::new().fg(C_DIM)));
    lines.push(Line::styled("  n new net   J join", Style::new().fg(C_DIM)));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_main(f: &mut Frame, area: Rect, app: &App) {
    let Some(net) = app.cur_net() else {
        f.render_widget(Paragraph::new("no networks"), area);
        return;
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    draw_net_header(f, rows[0], net);
    draw_tabline(f, rows[1], app.tab);

    match app.tab {
        Tab::Peers => draw_peers(f, rows[2], net, app.sel_peer),
        Tab::Transport => draw_transport(f, rows[2], app.cur_peer()),
        Tab::History => draw_history(f, rows[2], app),
    }
}

fn draw_net_header(f: &mut Frame, area: Rect, net: &NetSnap) {
    let l1 = Line::from(vec![
        Span::styled(trunc(&net.name, 24), Style::new().fg(C_ACCENT).bold()),
        Span::styled(format!("   role {}", net.role), Style::new().fg(C_DIM)),
    ]);
    let l2 = Line::from(vec![Span::styled(
        format!("subnet {}   tun {}   my {}", dash(&net.subnet), dash(&net.tun_name), net.my_ip),
        Style::new().fg(C_DIM),
    )]);
    f.render_widget(Paragraph::new(vec![l1, l2]), area);
}

fn draw_tabline(f: &mut Frame, area: Rect, cur: Tab) {
    let mut spans = Vec::new();
    for (i, t) in Tab::all().iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::new().fg(C_DIM)));
        }
        let style = if *t == cur {
            Style::new().fg(Color::Black).bg(C_ACCENT).bold()
        } else {
            Style::new().fg(C_DIM)
        };
        spans.push(Span::styled(format!(" {} ", t.title()), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_peers(f: &mut Frame, area: Rect, net: &NetSnap, sel: usize) {
    let header = Row::new(["", "host", "ip", "via", "rtt", "last"])
        .style(Style::new().fg(C_DIM).bold());
    let now = unix_now();
    let rows: Vec<Row> = net
        .peers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (mark, mcol) = peer_marker(p);
            let (via, vcol) = match &p.conn {
                Some(c) => conn_type_str(&c.conn_type),
                None => ("--", C_DIM),
            };
            let rtt = p.conn.as_ref().and_then(|c| c.rtt_ms);
            let r = Row::new([
                Cellish(mark.to_string(), mcol),
                Cellish(trunc(&p.hostname, 12), Color::Reset),
                Cellish(p.ip.clone(), C_DIM),
                Cellish(via.to_string(), vcol),
                Cellish(fmt_rtt(rtt), color_rtt(rtt)),
                Cellish(fmt_ago(p.last_seen, now), C_DIM),
            ]
            .map(Cell::from));
            if i == sel {
                row_selected(r)
            } else {
                r
            }
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Length(13),
        Constraint::Length(16),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(6),
    ];
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    f.render_widget(table, area);
}

fn draw_transport(f: &mut Frame, area: Rect, peer: Option<&PeerSnap>) {
    let Some(p) = peer else {
        f.render_widget(Paragraph::new(Span::styled("no peer selected", Style::new().fg(C_DIM))), area);
        return;
    };
    let Some(c) = &p.conn else {
        f.render_widget(
            Paragraph::new(Span::styled(format!("{} is not connected", p.hostname), Style::new().fg(C_DIM))),
            area,
        );
        return;
    };

    let (via, vcol) = conn_type_str(&c.conn_type);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(trunc(&p.hostname, 24), Style::new().bold()),
            Span::styled(format!("   {}", p.ip), Style::new().fg(C_DIM)),
        ]),
        Line::from(vec![
            Span::styled("transport  ", Style::new().fg(C_DIM)),
            Span::styled(via, Style::new().fg(vcol).bold()),
            Span::styled(
                format!(
                    "   rtt {}   mtu {}",
                    fmt_rtt(c.rtt_ms),
                    c.max_datagram_size.map(|m| m.to_string()).unwrap_or_else(|| "-".into()),
                ),
                Style::new().fg(C_DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("traffic    ", Style::new().fg(C_DIM)),
            Span::styled(
                format!(
                    "tx {}  rx {}  lost {}",
                    fmt_bytes(c.bytes_tx),
                    fmt_bytes(c.bytes_rx),
                    c.lost_packets
                ),
                Style::new().fg(C_DIM),
            ),
        ]),
    ];
    if let Some(remote) = &c.remote_addr {
        lines.push(Line::from(vec![
            Span::styled("remote     ", Style::new().fg(C_DIM)),
            Span::raw(remote.clone()),
        ]));
    }
    if let Some(v) = &c.via_detail {
        lines.push(Line::from(vec![
            Span::styled("why relay  ", Style::new().fg(C_DIM)),
            Span::styled(via_detail_str(v), Style::new().fg(C_WARN)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("candidate paths", Style::new().fg(C_DIM).bold()));
    if c.paths.is_empty() {
        lines.push(Line::styled("  (none reported)", Style::new().fg(C_DIM)));
    } else {
        for path in &c.paths {
            let (pvia, pcol) = conn_type_str(&path.conn_type);
            let sel = if path.is_selected { "▸" } else { " " };
            let flags = format!(
                "{}{}",
                if path.in_subnet { "" } else { " bled" },
                if path.has_activity { "" } else { " idle" },
            );
            let mtu = path.current_mtu.map(|m| format!("  mtu {m}")).unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(format!(" {sel} "), Style::new().fg(C_OK)),
                Span::styled(format!("{:<7}", pvia), Style::new().fg(pcol)),
                Span::raw(trunc(&path.remote_addr, 30)),
                Span::styled(format!("  {}", fmt_rtt(path.rtt_ms)), Style::new().fg(color_rtt(path.rtt_ms))),
                Span::styled(mtu, Style::new().fg(C_DIM)),
                Span::styled(flags, Style::new().fg(C_WARN)),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn draw_history(f: &mut Frame, area: Rect, app: &App) {
    let Some(p) = app.cur_peer() else {
        f.render_widget(Paragraph::new(Span::styled("no peer selected", Style::new().fg(C_DIM))), area);
        return;
    };
    let Some(hist) = app.history.get(&p.key) else {
        f.render_widget(
            Paragraph::new(Span::styled("collecting history…", Style::new().fg(C_DIM))),
            area,
        );
        return;
    };

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
        .split(area);

    spark(f, parts[0], "rtt (ms)", &hist.rtt, C_ACCENT, |v| v.to_string());
    spark(f, parts[1], "tx", &hist.tx_rate, C_OK, |v| fmt_rate(v));
    spark(f, parts[2], "rx", &hist.rx_rate, C_WARN, |v| fmt_rate(v));
}

fn spark(
    f: &mut Frame,
    area: Rect,
    label: &str,
    data: &std::collections::VecDeque<u64>,
    color: Color,
    fmt_cur: impl Fn(u64) -> String,
) {
    let v: Vec<u64> = data.iter().copied().collect();
    let cur = v.last().copied().unwrap_or(0);
    let max = v.iter().copied().max().unwrap_or(0);
    let title = format!(" {label}   now {}   peak {} ", fmt_cur(cur), fmt_cur(max));
    let block = Block::default().borders(Borders::ALL).title(title).title_style(Style::new().fg(C_DIM));
    let sk = Sparkline::default().block(block).data(v).max(max.max(1)).style(Style::new().fg(color));
    f.render_widget(sk, area);
}

fn draw_down(f: &mut Frame, area: Rect, app: &App) {
    let reason = app.down_reason.clone().unwrap_or_else(|| "unreachable".into());
    let lines = vec![
        Line::raw(""),
        Line::styled("tetron daemon not reachable", Style::new().fg(C_BAD).bold()),
        Line::raw(""),
        Line::styled(reason, Style::new().fg(C_DIM)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("press ", Style::new().fg(C_DIM)),
            Span::styled("s", Style::new().fg(C_ACCENT).bold()),
            Span::styled(" to run: ", Style::new().fg(C_DIM)),
            Span::styled("systemctl start tetron.service", Style::new().fg(C_ACCENT)),
        ]),
        Line::raw(""),
        Line::styled("retrying every 2s…", Style::new().fg(C_DIM).italic()),
    ];
    let p = Paragraph::new(lines).alignment(Alignment::Center).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, centered(area, 60, 11));
}

fn draw_help(f: &mut Frame) {
    let key = |k: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {k:<12}"), Style::new().fg(C_ACCENT)),
            Span::styled(d, Style::new()),
        ])
    };
    let lines = vec![
        Line::styled(" tetron-tui — keys ", Style::new().fg(C_ACCENT).bold()),
        Line::raw(""),
        key("↑/↓ j/k", "select peer"),
        key("←/→ h/l", "select network"),
        key("Tab 1 2 3", "switch pane"),
        Line::raw(""),
        key("c / d", "connect / standby selected network"),
        key("C / D", "connect / standby ALL networks"),
        key("i", "create invite (shows code + QR)"),
        key("n", "new network"),
        key("J", "join by invite code"),
        key("S", "stop daemon  ·  s: start (when down)"),
        Line::raw(""),
        key("? ", "this help      q / Esc  quit"),
        Line::raw(""),
        Line::styled("  press any key to close", Style::new().fg(C_DIM)),
    ];
    let n = lines.len() as u16 + 2;
    let area = centered(f.area(), 56, n);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_input(f: &mut Frame, kind: InputKind, buf: &str) {
    let (title, hint) = match kind {
        InputKind::Join => (" Join by invite code ", "paste the code, then Enter"),
        InputKind::Create => (" New network ", "type a name, then Enter"),
    };
    let area = centered(f.area(), 64, 6);
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(title).title_style(Style::new().fg(C_ACCENT).bold());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    // Field (truncated from the left so the caret end stays visible).
    let shown = tail(buf, rows[0].width.saturating_sub(2) as usize);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("> ", Style::new().fg(C_ACCENT)), Span::raw(shown.clone())])),
        rows[0],
    );
    f.render_widget(Paragraph::new(Span::styled(hint, Style::new().fg(C_DIM))), rows[2]);
    let cx = rows[0].x + 2 + shown.chars().count() as u16;
    f.set_cursor_position((cx.min(rows[0].x + rows[0].width.saturating_sub(1)), rows[0].y));
}

fn draw_invite(f: &mut Frame, network: &str, code: &str, id: &str, qr: &str) {
    let qr_w = qr.lines().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let qr_h = qr.lines().count() as u16;
    let w = qr_w.max(44).min(f.area().width.saturating_sub(2)) + 4;
    let h = qr_h + 7;
    let area = centered(f.area(), w, h);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Invite · {network} "))
        .title_style(Style::new().fg(C_ACCENT).bold());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("code  ", Style::new().fg(C_DIM)),
            Span::styled(code.to_string(), Style::new().fg(C_OK).bold()),
        ]),
        Line::from(vec![
            Span::styled("id    ", Style::new().fg(C_DIM)),
            Span::raw(id.to_string()),
        ]),
        Line::raw(""),
    ];
    for l in qr.lines() {
        lines.push(Line::raw(l.to_string()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("scan the QR or copy the code · Esc to close", Style::new().fg(C_DIM)));
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false }), inner);
}

fn draw_confirm(f: &mut Frame) {
    let area = centered(f.area(), 54, 7);
    f.render_widget(Clear, area);
    let lines = vec![
        Line::raw(""),
        Line::styled("Stop the tetron daemon?", Style::new().fg(C_BAD).bold()),
        Line::styled("This severs this UI's control socket.", Style::new().fg(C_DIM)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y", Style::new().fg(C_ACCENT).bold()),
            Span::styled(" stop    ", Style::new().fg(C_DIM)),
            Span::styled("n/Esc", Style::new().fg(C_ACCENT).bold()),
            Span::styled(" cancel", Style::new().fg(C_DIM)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

// --- small helpers ---------------------------------------------------------

/// A (text, color) pair that becomes a styled table `Cell`.
struct Cellish(String, Color);
impl From<Cellish> for Cell<'static> {
    fn from(c: Cellish) -> Self {
        Cell::from(c.0).style(Style::new().fg(c.1))
    }
}

fn row_selected(r: Row<'static>) -> Row<'static> {
    r.style(Style::new().bg(Color::Rgb(40, 44, 52)))
}

fn peer_marker(p: &PeerSnap) -> (&'static str, Color) {
    if p.is_coordinator {
        ("◆", C_ACCENT)
    } else if p.conn.is_some() {
        ("●", C_OK)
    } else {
        ("○", C_DIM)
    }
}

fn conn_type_str(t: &ConnType) -> (&'static str, Color) {
    match t {
        ConnType::Direct => ("direct", C_OK),
        ConnType::Relay => ("relay", C_WARN),
        ConnType::Tor => ("tor", Color::Magenta),
        ConnType::Veilid => ("veilid", C_ACCENT),
        ConnType::Unknown => ("?", C_DIM),
    }
}

fn via_detail_str(v: &ViaDetail) -> &'static str {
    match v {
        ViaDetail::NoDirectCandidate => "no direct candidate",
        ViaDetail::DirectUnvalidated => "direct candidate unvalidated",
        ViaDetail::DirectBled => "direct candidate excluded (overlay-bled)",
    }
}

fn color_rtt(rtt: Option<f64>) -> Color {
    match rtt {
        None => C_DIM,
        Some(r) if r < 20.0 => C_OK,
        Some(r) if r < 80.0 => C_WARN,
        Some(_) => C_BAD,
    }
}

fn fmt_rtt(rtt: Option<f64>) -> String {
    match rtt {
        Some(r) => format!("{r:.1}"),
        None => "-".into(),
    }
}

fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b}{}", U[0])
    } else {
        format!("{v:.1}{}", U[i])
    }
}

fn fmt_rate(bps: u64) -> String {
    format!("{}/s", fmt_bytes(bps))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn fmt_ago(ts: Option<u64>, now: u64) -> String {
    let Some(t) = ts else { return "-".into() };
    let d = now.saturating_sub(t);
    if d < 5 {
        "now".into()
    } else if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86400)
    }
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Keep the last `max` chars (so a long input stays caret-visible).
fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        s.chars().skip(n - max).collect()
    }
}

fn dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

/// A centered rect of the given size (clamped to `area`).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect { x, y, width: w, height: h }
}

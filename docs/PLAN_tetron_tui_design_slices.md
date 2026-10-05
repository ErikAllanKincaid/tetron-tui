# PLAN: tetron-tui design and slices

A rich, interactive terminal UI for the tetron P2P mesh VPN. Standalone binary in its own repo (`~/code/tetron-tui`), driven over the tetron daemon's local IPC socket. Usable at the local console and over plain SSH, with or without tetron-wm. Keyboard-first, mouse-capable. btop/htop-grade visuals.

Status: design agreed 2026-10-04. No code written yet. This document is the contract to build against.

## Why a separate binary

tetron is a standalone product and ships preinstalled in TetronOS. A TUI that works over plain SSH to any tetron box is useful with or without the window manager. tetron-wm's slice-3 "Open terminal" row changes from launching `sh -lc 'tetron status; exec $SHELL'` to launching `tetron-tui` directly, so the menu-driven panel supersedes the raw-status terminal.

Cost accepted: `tetron-proto` is pinned by git rev (as tetron-wm does), which drags its heavy transitive tree. An interactive async TUI pulls in tokio regardless, so that tree is not wasted here the way it would be in a one-shot poller. Splitting the pure wire-types into a leaf crate upstream (no tokio/reqwest) is still worth requesting, but it is not a blocker.

## Stack

- ratatui + crossterm (keyboard and mouse, truecolor) for rendering and input.
- tokio for the event loop and the async tetron-proto client.
- tetron-proto (pinned git rev, currently v0.14.0 / rev 135a365e) for the wire types and codec.
- qrcode crate rendered to unicode half-blocks for invite tickets (no image dependency).

Client approach: adopt tetron-proto's async tokio `Framed` / `MsgpackCodec` helpers. The one-shot sync framing used in `tetron-wm/src/system/tetron.rs` was right for the WM poller but wrong for a live event loop. If the async helpers prove awkward, the sync framing is a documented fallback: 4-byte big-endian length prefix plus `rmp_serde::to_vec_named` body, decode with `from_slice`.

## Wire facts (already proven by tetron-wm v1; do not re-derive)

- Socket path: `tetron_proto::ipc::socket_path()` (honors `TETRON_SOCKET_PATH`, else `/var/run/tetron/tetron.sock` on Linux). Mode `srw-rw-rw-`, user-reachable.
- Frame: 4-byte big-endian length prefix + msgpack body. `MsgpackCodec = LengthDelimitedCodec(length_field=4) + msgpack`.
- `IpcMessage` set: `Status` -> `StatusResponse{active, networks}`; `Resume{hostname, network}`; `Standby{network}`; `Sync`; `Create`; `Join{network_key, alias, hostname, transport, invite, force}`; `Leave{network, force}`; invite create/list; `Admin`(add/kick); `Kick`; nuke/second-proposal; firewall; `Shutdown`; `Ok{message}` / `Error`.

### Data the status reply carries (richer than expected)

- `NetworkStatus`: network, role, my_ip, my_ipv6, my_hostname, network_key, member_count, peers, tun_name, nuke_proposals, active, subnet, nuke_consensus_threshold.
- `PeerStatus`: endpoint_id, ip, ipv6, hostname, connection, is_coordinator, last_seen.
- `ConnectionInfo`: conn_type, remote_addr, rtt_ms, bytes_tx, bytes_rx, datagrams_tx, datagrams_rx, lost_packets, max_datagram_size, paths (every candidate path + a why-not-direct reason), and error counters malformed, backpressure, spoof, fragmentation_failed.
- `ConnType`: Direct / Relay / Tor / Veilid / Unknown (per connection).
- `TransportMode`: Default / Tor / Veilid (per-network preference, settable at Create/Join).

## Layout

Three regions. Left sidebar lists networks and action keys. Main area is a tab view for the selected network. Top and bottom bars show global status and hints.

```
+- tetron ------------------------ ⇄ active · 2 net · 3 peers -- 14:22 -+
| Networks          |  home-lab                    coordinator: you     |
| > home-lab   ●3/4 |  subnet 100.64.0.0/24   tun tet0   my 100.64.0.2  |
|   office     ○0/2 | +- Peers --------------------------------------+  |
|                   | | host        ip          via    rtt   last    |  |
| [c]onnect         | | ● nas       100.64.0.3  direct  1.2  now     |  |
| [d]isconnect      | | ● phone     100.64.0.4  relay  48    now     |  |
| [i]nvite          | | ○ laptop    100.64.0.5  --      --   3m      |  |
| [j]oin            | | ◆ gw(coord) 100.64.0.1  direct  0.8  now     |  |
| [n]ew network     | +----------------------------------------------+  |
| [a]dmin           |  tabs: Peers | Transport | History | Invite |Admin|
| [S]top daemon     |                                                    |
+- q quit · ? help ----------------------- refresh 1s · daemon ok ------+
```

## Tabs / panes

- **Peers** (default): live table, one row per peer. Columns host, ip, via (conn_type), rtt, last-seen, role marker. Sortable. btop-style color by health.
- **Transport**: per-peer transport detail. Current transport + conn_type, live MTU (`max_datagram_size`), and the full `paths` candidate list with the why-not-direct reason. Per-network `TransportMode` shown and settable (Default/Tor/Veilid). This is the network-engineering view the tray's single dot cannot give.
- **History**: btop-style braille charts. tx/rx throughput (rate derived from `bytes_tx/rx` deltas between polls) and an rtt series, per peer or aggregated per network. Ring buffer, 120 to 300 samples at 1s, configurable. Also surfaces error counters (lost, malformed, backpressure, spoof, fragmentation_failed) as a diagnostics strip.
- **Invite**: create a ticket with options. Ticket shown as selectable text and as a terminal QR (unicode half-blocks). List outstanding invites.
- **Admin** (gated on role == coordinator): add / kick members, nuke-consensus and second-a-proposal, firewall rules, leave network.

Join-by-code, new-network, and transport-mode change are modal popups, not tabs.

## Metrics and visual language (htop/btop)

- Gauges for rtt and throughput with green -> yellow -> red gradient by health.
- Braille line charts for history (ratatui Chart/Sparkline/Canvas).
- Truecolor throughout, works at the local console (kmscon) and over SSH.
- Diagnostics strip from the proto error counters, detail no web dashboard exposes.

## Daemon lifecycle (Stop / Start) vs data plane (Resume / Standby)

Two different axes, both present.

- **Resume / Standby** is the data-plane toggle: TUN link up vs on standby, control plane stays live. Per-network `●/○` plus a global toggle. Sends `Resume{hostname, network}` / `Standby{network}`. Already proven in tetron-wm slice 2.
- **Stop / Start** is daemon lifecycle. Stop sends `IpcMessage::Shutdown` (graceful, no privilege, over the socket). Stopping severs the TUI's own control socket, so the TUI then drops to a "daemon down" screen whose only action is Start. Start shells out to `systemctl start tetron.service` on TetronOS (may require privilege / polkit; a plain-SSH user without rights sees the command and the error). A Restart convenience does Shutdown then the systemctl start.

## Parity with tetron-webui

Near-full parity on core, exceeds on live metrics, lags only on board-discovery and add-ons (lowest value over SSH).

| Area | webui | tetron-tui |
| --- | --- | --- |
| Live status / peer list | yes | yes (streaming, richer) |
| Invite create + QR | yes (canvas) | yes (unicode QR + text) |
| Join by ticket | yes | yes |
| Create network | yes | yes |
| Resume / Standby | yes | yes |
| Stop / Start daemon | partial | yes (Shutdown + systemctl) |
| Transport mode + paths | partial | yes (full paths + why-not-direct) |
| History charts | no (snapshot poll) | yes (owns history) |
| Admin / kick / consensus | yes | yes |
| Firewall rules | yes | yes |
| Board discovery, add-ons | yes | defer (slice D or keep carbonyl escape-hatch) |

webui's live data is `/api/status` JSON, the same daemon data tetron-tui reads over tetron-proto. The dashboard adds no backend the TUI lacks; it is a JS frontend over the same daemon.

## Build slices (mirror tetron-wm v1 slicing: read-only first, then actions)

- **Slice A (read-only)**: event loop, async client, status poll + reconnect, sidebar + Peers tab + Transport tab + History charts. The "daemon down" screen. No actions that change state. This proves the client and the whole visual language.
- **Slice B (core actions)**: Resume/Standby per-network and global, Invite create with QR, Join by code, Create network, Stop/Start daemon.
- **Slice C (admin)**: add/kick, nuke-consensus and second-proposal, firewall rules, leave network. Role-gated.
- **Slice D (optional)**: board discovery and add-ons, or a documented decision to leave those to the carbonyl dashboard permanently.

## tetron-wm integration

Once slice A runs, change tetron-wm's slice-3 "Open terminal" popover row to launch `tetron-tui` instead of the raw status shell. Keep a fallback to the raw shell if the binary is absent.

## Reference implementations to lift from

- `~/code/tetron/tetron-systray/src`: status mapping, poll cadence.
- `~/code/tetron-wm/src/system/tetron.rs` and `src/tray.rs`: sync wire framing (fallback) and popover model.
- `~/code/tetron/tetron-webui` (`static/app.js`): feature inventory.
- `~/code/tetron/src`: the tetron CLI, for exact subcommand flags behind each IpcMessage.

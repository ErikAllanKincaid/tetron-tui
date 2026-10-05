//! Mutating actions (slice B). Each runs on its own short-lived daemon
//! connection, independent of the status poll loop, so a slow activation can
//! never stall rendering. Results flow back to the UI as [`ActionResult`].

use tetron_proto::ipc::{self, IpcMessage};
use tetron_proto::{invite, GroupMode};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// A user-requested change, pushed from the UI.
#[derive(Clone)]
pub enum Action {
    /// Resume (`active`) or Standby. `network = None` is daemon-wide.
    SetActive { network: Option<String>, active: bool },
    /// Mint a single-use invite key for a network (coordinator-only).
    InviteCreate { network: String },
    /// Join a network from an invite code (decoded client-side).
    Join { code: String },
    /// Create a new network with the given display name.
    Create { name: String },
    /// Gracefully stop the daemon (severs our own socket).
    Shutdown,
    /// Start the daemon via its systemd unit.
    StartDaemon,
}

/// The outcome of an [`Action`], reported back to the UI.
#[derive(Clone)]
pub enum ActionResult {
    /// A transient success message (toast).
    Ok(String),
    /// A transient error message (toast).
    Err(String),
    /// A freshly minted invite to show as text + QR.
    Invite { network: String, code: String, id: String },
}

/// Drain the action queue, running each on its own task.
pub async fn run(mut rx: UnboundedReceiver<Action>, tx: UnboundedSender<ActionResult>) {
    while let Some(a) = rx.recv().await {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(exec(a).await);
        });
    }
}

async fn exec(a: Action) -> ActionResult {
    match a {
        Action::StartDaemon => start_daemon().await,

        Action::SetActive { network, active } => {
            let label = network.clone().unwrap_or_else(|| "all networks".into());
            let msg = if active {
                IpcMessage::Resume { hostname: None, network }
            } else {
                IpcMessage::Standby { network }
            };
            match round_trip(msg).await {
                Ok(_) => ActionResult::Ok(format!(
                    "{} {label}",
                    if active { "resumed" } else { "standby" }
                )),
                Err(e) => ActionResult::Err(e),
            }
        }

        Action::InviteCreate { network } => {
            let msg = IpcMessage::InviteCreate { network: network.clone(), expires: None };
            match round_trip(msg).await {
                Ok(IpcMessage::InviteCreated { invite_key, invite_id, .. }) => {
                    ActionResult::Invite { network, code: invite_key, id: invite_id }
                }
                Ok(IpcMessage::Error { message }) => ActionResult::Err(message),
                Ok(_) => ActionResult::Err("unexpected reply to invite".into()),
                Err(e) => ActionResult::Err(e),
            }
        }

        Action::Join { code } => {
            let (key, secret) = match invite::decode_invite_code(code.trim()) {
                Ok(v) => v,
                Err(e) => return ActionResult::Err(format!("bad invite code: {e}")),
            };
            let msg = IpcMessage::Join {
                network_key: key.to_string(),
                alias: None,
                hostname: None,
                transport: None,
                invite: Some(secret),
                force: false,
            };
            match round_trip(msg).await {
                Ok(IpcMessage::Joined { network, .. }) => ActionResult::Ok(format!("joined {network}")),
                Ok(IpcMessage::Error { message }) => ActionResult::Err(message),
                Ok(_) => ActionResult::Err("unexpected reply to join".into()),
                Err(e) => ActionResult::Err(e),
            }
        }

        Action::Create { name } => {
            let msg = IpcMessage::Create {
                mode: GroupMode::Restricted,
                network_name: Some(name),
                hostname: None,
                transport: None,
                subnet: None,
                nuke_consensus: None,
                force: false,
            };
            match round_trip(msg).await {
                Ok(IpcMessage::Created { network, .. }) => ActionResult::Ok(format!("created {network}")),
                Ok(IpcMessage::Error { message }) => ActionResult::Err(message),
                Ok(_) => ActionResult::Err("unexpected reply to create".into()),
                Err(e) => ActionResult::Err(e),
            }
        }

        Action::Shutdown => {
            // The daemon replies Ok then exits; a dropped connection on the
            // reply is also success (it went down, as asked).
            let _ = round_trip(IpcMessage::Shutdown).await;
            ActionResult::Ok("daemon stopping…".into())
        }
    }
}

/// One request/one reply on a fresh connection.
async fn round_trip(msg: IpcMessage) -> Result<IpcMessage, String> {
    let mut f = ipc::connect().await.map_err(|e| e.to_string())?;
    ipc::send(&mut f, msg).await.map_err(|e| e.to_string())?;
    ipc::recv(&mut f).await.map_err(|e| e.to_string())
}

async fn start_daemon() -> ActionResult {
    use tokio::process::Command;
    match Command::new("systemctl").arg("start").arg("tetron.service").output().await {
        Ok(o) if o.status.success() => ActionResult::Ok("started tetron.service".into()),
        Ok(o) => ActionResult::Err(format!("systemctl: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => ActionResult::Err(format!("systemctl: {e}")),
    }
}

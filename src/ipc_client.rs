//! Background status poller.
//!
//! The daemon's IPC server handles exactly one request per connection and then
//! closes it (`handle_ipc_client`: one recv, one send, done). So each poll
//! opens a fresh connection, sends one `Status`, reads the reply, and closes —
//! the same one-shot model the action worker uses. Any error (socket missing,
//! I/O) is reported as `Down` and retried on the next tick, so the UI shows a
//! live daemon-down state without the task ever exiting.

use crate::model::{map_status, ConnStatus, Snapshot};
use std::time::Duration;
use tetron_proto::ipc::{self, IpcMessage};
use tokio::sync::mpsc::Sender;

pub async fn run(tx: Sender<ConnStatus>) {
    loop {
        let status = match poll_once().await {
            Ok(snap) => ConnStatus::Up(snap),
            Err(e) => ConnStatus::Down(e),
        };
        // Receiver gone => UI exited; stop the task.
        if tx.send(status).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn poll_once() -> Result<Snapshot, String> {
    let mut f = ipc::connect().await.map_err(|e| e.to_string())?;
    ipc::send(&mut f, IpcMessage::Status).await.map_err(|e| e.to_string())?;
    let msg = ipc::recv(&mut f).await.map_err(|e| e.to_string())?;
    map_status(msg).ok_or_else(|| "unexpected reply to status".into())
}

//! Background task: hold one connection to the tetron daemon, poll `Status`
//! every second, and report each result to the UI over a channel. Any I/O
//! error drops the connection and retries, so the UI shows a live
//! "daemon down" state without the task ever exiting.

use crate::model::{map_status, ConnStatus};
use std::time::Duration;
use tetron_proto::ipc::{self, IpcMessage};
use tokio::sync::mpsc::Sender;

pub async fn run(tx: Sender<ConnStatus>) {
    loop {
        match ipc::connect().await {
            Ok(mut framed) => {
                loop {
                    if ipc::send(&mut framed, IpcMessage::Status).await.is_err() {
                        let _ = tx.send(ConnStatus::Down("send failed".into())).await;
                        break;
                    }
                    match ipc::recv(&mut framed).await {
                        Ok(msg) => {
                            if let Some(snap) = map_status(msg) {
                                // Receiver gone => UI exited; stop the task.
                                if tx.send(ConnStatus::Up(snap)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(ConnStatus::Down(e.to_string())).await;
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            Err(e) => {
                if tx.send(ConnStatus::Down(e.to_string())).await.is_err() {
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

//! PTY 原始终端（文档03：终端是可升降抽屉，属于每个视图）。
//! portable-pty 拉起 `adb -s <serial> shell`，输出经事件流推前端 xterm.js。
use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::Emitter;
use tokio::sync::Mutex;

static SEQ: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct TerminalInfo {
    pub id: u32,
    pub serial: String,
    pub created_at: String,
}

pub struct TerminalSession {
    pub id: u32,
    pub serial: String,
    writer: std::sync::Mutex<Box<dyn Write + Send>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalMgr {
    pub sessions: Mutex<HashMap<u32, TerminalSession>>,
}

pub async fn create(
    app: tauri::AppHandle,
    mgr: &TerminalMgr,
    serial: &str,
) -> Result<TerminalInfo, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 26,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("PTY 创建失败：{e}"))?;
    let mut cmd = CommandBuilder::new("adb");
    cmd.args(["-s", serial, "shell"]);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("adb shell 启动失败：{e}"))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("PTY reader 获取失败：{e}"))?;
    let writer = std::sync::Mutex::new(
        pair.master
            .take_writer()
            .map_err(|e| format!("PTY writer 获取失败：{e}"))?,
    );

    let id = SEQ.fetch_add(1, Ordering::SeqCst);
    let info = TerminalInfo {
        id,
        serial: serial.to_string(),
        created_at: chrono::Local::now().format("%H:%M:%S").to_string(),
    };

    // 读泵：PTY → 事件（base64，二进制安全）
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                        if app
                            .emit("terminal-out", json!({"id": id, "b64": b64}))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            let _ = app.emit("terminal-closed", json!({"id": id}));
        });
    }
    drop(pair.slave); // 部分平台需要释放 slave 句柄

    mgr.sessions.lock().await.insert(
        id,
        TerminalSession {
            id,
            serial: serial.to_string(),
            writer,
            master: pair.master,
            child,
        },
    );
    crate::audit::audit("terminal_create", serial, "done", "terminal-drawer", &format!("term#{id}"));
    Ok(info)
}

pub async fn write(mgr: &TerminalMgr, id: u32, data: &str) -> Result<(), String> {
    let s = mgr.sessions.lock().await;
    let t = s.get(&id).ok_or("终端会话不存在")?;
    let mut w = t.writer.lock().map_err(|_| "writer 忙")?;
    w.write_all(data.as_bytes())
        .map_err(|e| format!("写入失败：{e}"))?;
    w.flush().map_err(|e| format!("flush 失败：{e}"))
}

pub async fn resize(mgr: &TerminalMgr, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let s = mgr.sessions.lock().await;
    let t = s.get(&id).ok_or("终端会话不存在")?;
    t.master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("resize 失败：{e}"))
}

pub async fn close(app: tauri::AppHandle, mgr: &TerminalMgr, id: u32) -> Result<(), String> {
    let t = mgr.sessions.lock().await.remove(&id);
    match t {
        Some(mut s) => {
            let _ = s.child.kill();
            if let Ok(mut w) = s.writer.lock() {
                let _ = w.flush();
            }
            crate::audit::audit("terminal_close", &s.serial, "done", "terminal-drawer", &format!("term#{id}"));
            let _ = app.emit("terminal-closed", json!({"id": id}));
            Ok(())
        }
        None => Err("终端会话不存在".into()),
    }
}

pub async fn list(mgr: &TerminalMgr) -> Vec<TerminalInfo> {
    mgr.sessions
        .lock()
        .await
        .values()
        .map(|s| TerminalInfo {
            id: s.id,
            serial: s.serial.clone(),
            created_at: String::new(),
        })
        .collect()
}

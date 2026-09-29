use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// 关闭握手状态机（LovelyMem window_events.rs 模式）：
/// IDLE → WAITING（前端确认）→ SHUTTING；10s 无响应由看门狗保底强停。
pub const SHUTDOWN_IDLE: u8 = 0;
pub const SHUTDOWN_WAITING: u8 = 1;
pub const SHUTDOWN_SHUTTING: u8 = 2;

pub struct AppState {
    pub shutdown_phase: AtomicU8,
    #[allow(dead_code)] // 供 M1 会话诊断使用
    pub started: Instant,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            shutdown_phase: AtomicU8::new(SHUTDOWN_IDLE),
            started: Instant::now(),
        }
    }

    #[allow(dead_code)] // M1 清理流程启用
    pub fn is_shutting(&self) -> bool {
        self.shutdown_phase.load(Ordering::SeqCst) >= SHUTDOWN_SHUTTING
    }
}

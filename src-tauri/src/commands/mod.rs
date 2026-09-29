//! Tauri commands：按域分组注册（LovelyMem lib.rs 模式，避免巨型单文件）。
//! 注意：generate_handler! 必须用完整模块路径（`#[tauri::command]` 生成的
//! `__cmd__` 辅助宏只存在于定义命令的模块内，不随 `pub use` 重导出）。
pub mod adb_cmd;
pub mod app_cmd;
pub mod config_cmd;
pub mod doctor_cmd;
pub mod frida_cmd;
pub mod guard_cmd;

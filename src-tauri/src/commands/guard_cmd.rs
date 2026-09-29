use crate::guard::GuardVerdict;

/// UI 侧预检：给用户看「这个路径能不能写、为什么」。
/// 真正的写操作在后端各自入口再强制过 guard（双保险）。
#[tauri::command]
pub async fn guard_check_write(path: String) -> Result<GuardVerdict, String> {
    Ok(crate::guard::check_write(&path))
}

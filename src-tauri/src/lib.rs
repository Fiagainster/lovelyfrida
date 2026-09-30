mod audit;
mod backends;
mod commands;
mod config;
mod guard;
mod paths;
mod services;
mod state;
mod store;

use state::{AppState, SHUTDOWN_IDLE, SHUTDOWN_WAITING};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{Emitter, Manager, State};

pub fn run() {
    // 日志：logs/app.log（无轮转，M5 做环形清理）+ 开发期控制台
    init_logging();

    // 启动前置：目录布局 / 配置 / 数据库（失败不崩壳，记录错误供首启自检暴露）
    if let Err(e) = paths::ensure_layout() {
        tracing::error!("目录布局创建失败：{e}");
    }
    if let Err(e) = config::init() {
        tracing::error!("config.toml 加载失败：{e}");
    }
    if let Err(e) = store::init() {
        tracing::error!("cases.db 初始化失败：{e}");
    }
    tracing::info!("LovelyFrida 启动，根目录 = {}", paths::app_root().display());

    // 通道B sidecar 的 Python 解释器：环境变量 > PATH 上的 python
    let sidecar_python = std::env::var("LOVELYFRIDA_PYTHON")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "python".into());

    tauri::Builder::default()
        .manage(AppState::new())
        .manage(services::session::FridaState::new(sidecar_python))
        .manage(services::terminal::TerminalMgr::default())
        .manage(services::recorder::RecorderState::default())
        .manage(services::trace::TraceState::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_cmd::get_app_info,
            commands::app_cmd::confirm_close,
            commands::config_cmd::get_config,
            commands::config_cmd::update_config,
            commands::adb_cmd::resolve_adb,
            commands::adb_cmd::adb_devices,
            commands::adb_cmd::adb_connect,
            commands::adb_cmd::adb_self_heal,
            commands::doctor_cmd::doctor_run,
            commands::doctor_cmd::first_run_self_check,
            commands::guard_cmd::guard_check_write,
            commands::frida_cmd::frida_server_status,
            commands::frida_cmd::frida_server_install,
            commands::frida_cmd::frida_forward_setup,
            commands::frida_cmd::frida_processes,
            commands::frida_cmd::frida_session_attach,
            commands::frida_cmd::frida_session_detach,
            commands::frida_cmd::frida_session_status,
            commands::frida_cmd::frida_session_ping,
            commands::frida_cmd::frida_rpc,
            commands::terminal_cmd::terminal_create,
            commands::terminal_cmd::terminal_write,
            commands::terminal_cmd::terminal_resize,
            commands::terminal_cmd::terminal_close,
            commands::terminal_cmd::terminal_list,
            commands::terminal_cmd::recorder_list,
            commands::terminal_cmd::recorder_export,
            commands::terminal_cmd::recorder_clear,
        ])
        .on_window_event(|window, event| {
            // 关闭握手（LovelyMem 协议）：拦截 → 通知前端 → 10s 看门狗保底
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let state: State<AppState> = app.state();
                if state
                    .shutdown_phase
                    .compare_exchange(SHUTDOWN_IDLE, SHUTDOWN_WAITING, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    let handle = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(10));
                        let s: State<AppState> = handle.state();
                        if s.shutdown_phase.load(Ordering::SeqCst) == SHUTDOWN_WAITING {
                            tracing::warn!("关闭确认 10 秒无响应，执行保底关停");
                            graceful_shutdown(&handle);
                        }
                    });
                }
                let _ = app.emit("close-requested", ());
            }
        })
        .setup(|app| {
            // sidecar 事件 → 前端转发（message/detached/device_lost/spawn_added）
            let ev_handle = app.handle().clone();
            let frida = app.state::<services::session::FridaState>().channel.clone();
            let trace = std::sync::Arc::new(services::trace::TraceState::default());
            app.manage(trace.clone());
            tauri::async_runtime::spawn(async move {
                services::session::forward_events(ev_handle, frida, trace).await;
            });

            tauri::async_runtime::spawn(async move {
                match services::first_run::run().await {
                    Ok(report) => {
                        for i in &report.items {
                            if i.ok {
                                tracing::info!("[首启自检 {}] {} — {}", i.id, i.name, i.detail);
                            } else {
                                tracing::warn!("[首启自检 {}] {} — {}", i.id, i.name, i.detail);
                            }
                        }
                    }
                    Err(e) => tracing::error!("首启自检执行失败：{e}"),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("LovelyFrida 启动失败");
}

pub fn graceful_shutdown(app: &tauri::AppHandle) {
    let state: State<AppState> = app.state();
    state
        .shutdown_phase
        .store(state::SHUTDOWN_SHUTTING, Ordering::SeqCst);
    // M1：在此先 detach frida 会话、停止 frida-server/sidecar/PTY 子进程，限时清理
    audit::audit("shutdown", "app", "done", "close-handshake", "优雅关停");
    let _ = app.exit(0);
}

fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = std::fs::create_dir_all(paths::logs_dir());
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,lovelyfrida_lib=debug"));
    let file_appender = tracing_appender::rolling::never(paths::logs_dir(), "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // guard 需要活到进程结束（worker 线程 flushing）
    std::mem::forget(guard);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(non_blocking);
    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    #[cfg(debug_assertions)]
    let registry = registry.with(tracing_subscriber::fmt::layer());
    registry.init();
}

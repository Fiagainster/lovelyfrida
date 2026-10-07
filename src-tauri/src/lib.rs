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
    // 日志：logs/app.log 按天轮转（A4b：兑现 M5「环形清理」承诺为轮转语义）+ 开发期控制台
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

    // 通道B sidecar 启动方式（文档10 P4-1）：打包 exe 优先，开发态回退 python 源码
    let sidecar_python = std::env::var("LOVELYFRIDA_PYTHON")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "python".into());
    let sidecar_launch = paths::resolve_sidecar_launch(&sidecar_python);
    tracing::info!("[通道B] sidecar 启动方式：{}", sidecar_launch.label);
    // 通道选择（文档10 P3-4）：preferred_channel 驱动 attach 分支，UI 明示当前通道
    let preferred_channel = config::get().preferred_channel.clone();

    tauri::Builder::default()
        .manage(AppState::new())
        .manage(services::session::FridaState::new(
            sidecar_launch,
            preferred_channel,
        ))
        .manage(services::terminal::TerminalMgr::default())
        .manage(services::recorder::RecorderState::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_cmd::get_app_info,
            commands::app_cmd::confirm_close,
            commands::app_cmd::cancel_close,
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
            commands::frida_cmd::frida_server_fetch,
            commands::frida_cmd::frida_forward_setup,
            commands::frida_cmd::frida_processes,
            commands::frida_cmd::frida_session_attach,
            commands::frida_cmd::frida_session_detach,
            commands::frida_cmd::frida_session_status,
            commands::frida_cmd::frida_session_ping,
            commands::frida_cmd::frida_rpc,
            commands::frida_cmd::experiment_run,
            commands::frida_cmd::injection_run,
            commands::frida_cmd::crypto_reconstruct,
            commands::frida_cmd::brute_estimate,
            commands::frida_cmd::brute_run,
            commands::frida_cmd::brute_generate_c,
            commands::frida_cmd::ledger_add,
            commands::frida_cmd::ledger_list,
            commands::frida_cmd::ledger_delete,
            commands::frida_cmd::ledger_export_md,
            commands::frida_cmd::ledger_export_bundle,
            commands::frida_cmd::script_list,
            commands::frida_cmd::script_read,
            commands::frida_cmd::script_save,
            commands::frida_cmd::script_delete,
            commands::frida_cmd::profile_save,
            commands::frida_cmd::profile_list,
            commands::frida_cmd::profile_delete,
            commands::frida_cmd::dumps_list,
            commands::frida_cmd::history_sessions,
            commands::frida_cmd::history_runs,
            commands::frida_cmd::history_experiments,
            commands::frida_cmd::history_brute_jobs,
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
                    .compare_exchange(
                        SHUTDOWN_IDLE,
                        SHUTDOWN_WAITING,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    let handle = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(10));
                        let s: State<AppState> = handle.state();
                        // WAITING（用户一直没应答）或 SHUTTING（清理线程意外卡死）都保底强杀：
                        // graceful_shutdown 幂等，二次调用只会再拉一条限时清理线程 → 必然 exit
                        if s.shutdown_phase.load(Ordering::SeqCst) != SHUTDOWN_IDLE {
                            tracing::warn!("关闭确认 10 秒未完成，执行保底关停");
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

/// 优雅关停（关闭握手第三步）：限时清理（detach 会话 + 关 PTY）后必然退出。
///
/// ⚠️ 清理必须在独立 std 线程跑：confirm_close 是 async 命令（tokio worker 线程），
/// 在 worker 上 `block_on` 会 panic（"Cannot block the current thread from within a
/// runtime"），panic 把 phase 留在 SHUTTING → 旧看门狗条件（==WAITING）不再命中 →
/// 应用永久无法退出（真机联调后用户实测：只能任务管理器强杀）。
pub fn graceful_shutdown(app: &tauri::AppHandle) {
    let state: State<AppState> = app.state();
    let already = state
        .shutdown_phase
        .swap(state::SHUTDOWN_SHUTTING, Ordering::SeqCst)
        == state::SHUTDOWN_SHUTTING;
    if !already {
        audit::audit(
            "shutdown",
            "app",
            "done",
            "close-handshake",
            "优雅关停：限时清理",
        );
    }
    let handle = app.clone();
    // 清理任务丢回 tokio runtime 正常跑（绝不从外部线程 block_on：Handle::block_on
    // 在 runtime 外的线程上不驱动 timer/IO 驱动，8s 总闸自己永远到不了——真机实测卡死）。
    // 外部线程只做两件事：盯完成旗标（≤8s）→ exit(0)。exit 从任何线程调用都安全。
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_task = done.clone();
    tauri::async_runtime::spawn(async move {
        // 兑现 M1 注释承诺：每步各自带超时——卡死的子进程不拖住退出
        // ① frida 会话分离（unload agent + detach + trace 落盘收尾 + 落库收尾）；
        // ② PTY 终端逐个关闭（杀 adb shell 子进程）。
        // sidecar/adb 客户端进程由 kill_on_drop + 进程退出兜底；设备端 frida-server
        // 保留（取证工具不假设下一次连接环境，设备侧状态由会话链路自行探测/拉起）。
        let frida = handle.state::<services::session::FridaState>();
        let _ = tokio::time::timeout(
            Duration::from_secs(4),
            services::session::detach(&handle, &frida),
        )
        .await;
        let term = handle.state::<services::terminal::TerminalMgr>();
        let ids: Vec<u32> = term.sessions.lock().await.keys().copied().collect();
        for id in ids {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                services::terminal::close(handle.clone(), &term, id),
            )
            .await;
        }
        done_task.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let handle_exit = app.clone();
    let done_watch = done.clone();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        while !done_watch.load(std::sync::atomic::Ordering::SeqCst)
            && t0.elapsed() < Duration::from_secs(8)
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        handle_exit.exit(0);
    });
}

fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = std::fs::create_dir_all(paths::logs_dir());
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,lovelyfrida_lib=debug"));
    let file_appender = tracing_appender::rolling::daily(paths::logs_dir(), "app.log");
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

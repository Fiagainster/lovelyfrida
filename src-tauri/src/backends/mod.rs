//! 能力层 trait 定义区（文档02 架构纪律）：
//! 所有设备操作必须过 AdbBackend，所有 frida 操作必须过 FridaBackend。
//! M1 落实 FridaBackend trait 与通道 B/C 实现；M0 仅 AdbBackend。
pub mod adb;
pub mod frida;

// FridaBackend trait 面向 frida-node 完整 API 面设计（M1 实现）：
// devices / processes / spawn / resume / kill / attach(realm, persist_timeout)
// / enable_spawn_gating / child_gating / create_script / load / post / messages
// / rpc_call / detach / output / process_crashed。三通道（A/B/C）实现同一 trait，
// 切换策略 A→B→C 自动降级且必须 UI 明示。

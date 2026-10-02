//! 能力层 trait 定义区（文档02 架构纪律）：
//! 所有设备操作必须过 AdbBackend，所有 frida 操作必须过 FridaBackend。
//! M1 落实通道B；阶段③（文档10）补通道C（CLI 兜底，观测级降级）；通道A 保持远期。
pub mod adb;
pub mod frida;
pub mod frida_c;

// FridaBackend trait 面向 frida-node 完整 API 面设计（M1 实现）：
// devices / processes / spawn / resume / kill / attach(realm, persist_timeout)
// / enable_spawn_gating / child_gating / create_script / load / post / messages
// / rpc_call / detach / output / process_crashed。三通道（A/B/C）实现同一 trait，
// 切换策略 auto 先 B、B 不可用降级 C，且必须 UI 明示当前通道。
// 现状（文档10 阶段③）：B/C 以各自具体类型存在，attach 层做通道分支与明示；
// 完整 trait 统一推迟到通道 A 落地时一并收口（避免为两个实现预付抽象税）。

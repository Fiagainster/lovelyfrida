//! 应用服务层（文档02）：Doctor / SessionMgr(M1) / Injector(M3) / ProbeLab(M2)
//! / Experiment(M3) / Crypto(M4) / BruteOrch(M4) / Ledger(M5) / Diagnostics(M2) / Recorder(M1)。
pub mod brute;
pub mod crypto;
pub mod device_shell;
pub mod doctor;
pub mod experiment;
pub mod extras_svc;
pub mod first_run;
pub mod frida_fetch;
pub mod history;
pub mod injection;
pub mod ledger;
pub mod recorder;
pub mod session;
pub mod terminal;
pub mod trace;

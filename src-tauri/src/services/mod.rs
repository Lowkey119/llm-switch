//! Domain services shared by the CLI, the desktop IPC and the gateway.

pub mod agents_scan;
pub mod backend;
/// 随包内嵌的预设配置归档（zip）一键还原。
pub mod config_archive;
pub mod discover;
/// 引擎原生速率（切片 11）：halogen 计数器差分，TTL 合流缓存。
pub mod engine_rates;
pub mod ext_install;
pub mod halctl;
/// 参数下发：参数中心的模型层键值 → 各家后端自己的模型配置。
pub mod param_push;
/// 第三方 ponytail 插件：托管检出、上游双源检测、逐端安装清单。
pub mod ponytail;
pub mod pricing;
/// 快速面板 tab 配置（quickpanel.json）：结构、校验与原子写盘。
pub mod quickpanel;
pub mod remote;
pub mod script;
pub mod skill;
/// 系统资源监控（切片 12）：本机 macOS + 102 SSH 硬件指标，TTL 缓存。
pub mod sysmon;
pub mod task_queue;
pub mod update;

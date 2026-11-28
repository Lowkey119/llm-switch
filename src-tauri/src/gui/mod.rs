//! Desktop shell: Tauri 2 window + tray + single-instance, hosting the same
//! in-process gateway as the headless CLI. Closing the window hides it to the
//! tray; quitting happens from the tray menu.

pub mod floatwin;
pub mod i18n;
pub mod promptfloat;
pub mod rateline;
pub mod sysfloat;

use crate::config::Config;
use crate::gateway::{self, State};
use crate::store::Store;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalSize, RunEvent};

pub fn run(
    store: Arc<Store>,
    config: Arc<Config>,
    float_at_start: bool,
    // 换血/并行：`--swap` 或显式 `--port` 时不注册单实例锁——旧实例还在跑时
    // 新实例（不同端口）要能起来并行验证，换血脚本据此先起新、探活、再收旧。
    allow_parallel: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = gateway::State::new(store, config);

    let app = tauri::Builder::default();
    let app = if allow_parallel {
        app
    } else {
        app.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main(app);
        }))
    };
    let app = app
        .plugin(
            // 提示词速取面板的全局唤起快捷键（计划 2026-11-18）。快捷键在 setup 里
            // 按库存注册（可自定义）；这里只挂回调——按下即切换面板显隐。
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state() == ShortcutState::Pressed {
                        promptfloat::toggle(app);
                    }
                })
                .build(),
        )
        .plugin(
            // 浮窗的位置是自己算的（屏幕右上角），不能被上次会话的状态拽回来。
            // 速率线同理，而且它的标签是动态的（rate-<代理>），逐个列进 denylist
            // 永远列不全 —— 按前缀整族排除，位置由 rateline 自己记在 settings。
            // 资源小窗（sysfloat）也是自己记位置的，一并拉黑。提示词速取面板
            // （prompt）同样自记位置/尺寸（settings），不能被会话状态拽回。
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[floatwin::LABEL, sysfloat::LABEL, promptfloat::LABEL])
                .with_filter(|label| !label.starts_with(rateline::LABEL_PREFIX))
                .build(),
        )
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            crate::commands::panel::model_metrics,
            crate::commands::panel::refresh_backend,
            crate::commands::panel::check_port,
            crate::commands::panel::set_listen_port,
            crate::commands::panel::open_url,
            crate::commands::panel::open_internal,
            crate::commands::panel::copilot_get,
            crate::commands::panel::copilot_set,
            crate::commands::backend::list_backends,
            crate::commands::backend::console_flow,
            crate::commands::halctl::halctl_status,
            crate::commands::halctl::halctl_control,
            crate::commands::halctl::halctl_use_engine,
            crate::commands::halctl::halctl_logs,
            crate::commands::halctl::halctl_versions,
            crate::commands::halctl::halctl_switch,
            crate::commands::halctl::halctl_backup,
            crate::commands::halctl::halctl_restore,
            crate::commands::halctl::halctl_env,
            crate::commands::halctl::halctl_env_save,
            crate::commands::halctl::halctl_env_preset_save,
            crate::commands::halctl::halctl_env_preset_apply,
            crate::commands::halctl::halctl_env_preset_del,
            crate::commands::halctl::halctl_cfg_get,
            crate::commands::halctl::halctl_cfg_set,
            crate::commands::backend::upsert_backend,
            crate::commands::backend::set_backend_enabled,
            crate::commands::backend::set_backend_direct,
            crate::commands::backend::delete_backend,
            crate::commands::backend::probe_backend,
            crate::commands::backend::probe_models,
            crate::commands::backend::set_model_exposed,
            crate::commands::backend::set_model_accept,
            crate::commands::backend::set_model_kind,
            crate::commands::backend::model_meta_get,
            crate::commands::backend::model_groups,
            crate::commands::backend::model_meta_set,
            crate::commands::backend::model_price_set,
            crate::commands::backend::rename_model,
            crate::commands::backend::remove_model,
            crate::commands::backend::get_status,
            crate::commands::backend::set_paused,
            crate::commands::params::list_param_overrides,
            crate::commands::params::upsert_param_override,
            crate::commands::params::delete_param_override,
            crate::commands::params::get_param_catalog,
            crate::commands::params::get_backend_capabilities,
            crate::commands::prompt::prompt_list,
            crate::commands::prompt::prompt_create,
            crate::commands::prompt::prompt_update,
            crate::commands::prompt::prompt_delete,
            crate::commands::prompt::prompt_use,
            crate::commands::prompt::prompt_set_visible,
            crate::commands::prompt::prompt_shape,
            crate::commands::prompt::prompt_expand,
            crate::commands::prompt::prompt_hotkey_get,
            crate::commands::prompt::prompt_hotkey_set,
            crate::commands::prompt::prompt_mode_get,
            crate::commands::prompt::prompt_mode_set,
            crate::commands::prompt::prompt_inject,
            crate::commands::prompt::prompt_apps,
            crate::commands::prompt::prompt_target_set,
            crate::commands::prompt::prompt_target,
            crate::commands::prompt::prompt_a11y_status,
            crate::commands::prompt::prompt_a11y_ask,
            crate::commands::prompt::prompt_a11y_open,
            crate::commands::prompt::prompt_a11y_reset,
            crate::commands::prompt::quickpanel_config_get,
            crate::commands::prompt::quickpanel_config_set,
            crate::commands::prompt::quickpanel_run_action,
            crate::commands::prompt::remote_macro_run,
            crate::commands::prompt::remote_action_run,
            crate::commands::window::main_min_size,
            crate::commands::prompt::prompt_recent_get,
            crate::commands::prompt::prompt_recent_add,
            crate::commands::prompt::prompt_recent_clear,
            crate::commands::prompt::prompt_recent_remove,
            crate::commands::prompt::prompt_autohide_get,
            crate::commands::prompt::prompt_autohide_set,
            crate::commands::logs::logs_query,
            crate::commands::logs::logs_purge,
            crate::commands::logs::logs_export,
            crate::commands::logs::logs_facets,
            crate::commands::logs::get_log_level,
            crate::commands::logs::set_log_level,
            crate::commands::logs::logs_retention,
            crate::commands::logs::set_logs_retention,
            crate::commands::metrics::get_metrics,
            crate::commands::metrics::get_model_rates,
            crate::commands::metrics::get_backend_model_rates,
            crate::commands::metrics::get_engine_rates,
            crate::commands::metrics::auto_float_get,
            crate::commands::metrics::auto_float_set,
            crate::commands::metrics::get_rates,
            crate::commands::cache::get_cache_stats,
            crate::commands::cache::get_backend_telemetry,
            crate::commands::cache::purge_cache,
            crate::commands::cache::list_cache_policies,
            crate::commands::cache::get_cache_tier,
            crate::commands::cache::save_cache_tier,
            crate::commands::cache::set_cache_policy,
            crate::commands::cache::delete_cache_policy,
            crate::commands::task::list_tasks,
            crate::commands::update::update_check,
            crate::commands::update::update_download,
            crate::commands::update::update_install,
            crate::commands::task::task_unread,
            crate::commands::task::mark_tasks_seen,
            crate::commands::usage::usage_daily,
            crate::commands::usage::usage_records,
            crate::commands::routing::get_routing,
            crate::commands::routing::set_routing,
            crate::commands::ext_install::ext_status,
            crate::commands::ext_install::ext_install,
            crate::commands::ext_install::ext_uninstall,
            crate::commands::ext_install::ponytail_status,
            crate::commands::ext_install::ponytail_check,
            crate::commands::ext_install::ponytail_update,
            crate::commands::skill::get_skills,
            crate::commands::skill::scan_skills,
            crate::commands::skill::resync_skills,
            crate::commands::skill::set_skill_app_enabled,
            crate::commands::skill::uninstall_skill,
            crate::commands::skill::list_skill_backups,
            crate::commands::skill::backup_skills_now,
            crate::commands::skill::delete_skill_backup,
            crate::commands::skill::restore_skill_backup,
            crate::commands::skill::get_skill_settings,
            crate::commands::skill::set_skill_sync_method,
            crate::commands::skill::install_skill,
            crate::commands::skill::import_skills_zip,
            crate::commands::skill::scan_unmanaged_skills,
            crate::commands::skill::claim_skill,
            crate::commands::skill::list_skill_repos,
            crate::commands::skill::add_skill_repo,
            crate::commands::skill::delete_skill_repo,
            crate::commands::skill::set_skill_repo_enabled,
            crate::commands::skill::discover_skills,
            crate::commands::skill::search_skills,
            crate::commands::skill::check_skill_updates,
            crate::commands::skill::update_skill,
            crate::commands::skill::read_skill_doc,
            crate::commands::backend::set_backend_console,
            crate::commands::backend::run_backend_script,
            crate::commands::usage::usage_cost,
            crate::commands::usage::usage_stats,
            crate::commands::usage::usage_media,
            crate::commands::usage::usage_retention,
            crate::commands::usage::set_usage_retention,
            crate::commands::takeover::takeover_status,
            crate::commands::takeover::takeover_apply,
            crate::commands::takeover::takeover_revert,
            crate::commands::takeover::takeover_backups,
            crate::commands::takeover::takeover_restore_backup,
            crate::commands::takeover::takeover_delete_backup,
            crate::commands::takeover::get_takeover_settings,
            crate::commands::takeover::set_takeover_settings,
            crate::commands::takeover::takeover_files,
            crate::commands::takeover::takeover_read_file,
            crate::commands::takeover::takeover_write_file,
            crate::commands::takeover::takeover_model_get,
            crate::commands::takeover::takeover_model_set,
            crate::commands::takeover::claude_switches_get,
            crate::commands::takeover::claude_switch_set,
            crate::commands::takeover::matrix_get,
            crate::commands::takeover::matrix_cell,
            crate::commands::takeover::preset_status,
            crate::commands::takeover::preset_restore,
            crate::commands::takeover::client_list,
            crate::commands::takeover::takeover_path_set,
            crate::commands::takeover::takeover_path_reset,
            crate::commands::takeover::agents_set_enabled,
            crate::commands::agents::agents_scan,
            crate::commands::agents::agent_install,
            crate::commands::config_io::config_export,
            crate::commands::config_io::config_import,
            crate::commands::halctl::halctl_image_del,
            crate::commands::config_archive::config_archive_info,
            crate::commands::config_archive::config_archive_backup,
            crate::commands::config_archive::config_archive_backup_to,
            crate::commands::config_archive::config_archive_restore,
            crate::commands::config_archive::config_archive_restore_from,
            crate::commands::float::float_toggle,
            crate::commands::ui::ui_lang_get,
            crate::commands::ui::ui_lang_set,
            crate::commands::float::float_set_visible,
            crate::commands::float::float_is_visible,
            // 形状相关的四个入口必须一并注册：漏了它们，前端每次调用都被
            // 「command not found」拒掉（前端一律 .catch 吞掉），表现就是
            // 「双击完全没反应、窗口永远不收身」。
            crate::commands::float::float_state,
            crate::commands::float::float_double_click,
            crate::commands::float::float_fit,
            crate::commands::float::float_zoom_resize,
            crate::commands::float::rateline_set,
            crate::commands::float::rateline_list,
            crate::commands::float::rateline_fit,
            crate::commands::float::rateline_auto_get,
            crate::commands::float::rateline_auto_set,
            crate::commands::sysmon::get_sys_stats,
            crate::commands::sysmon::sysfloat_get_visible,
            crate::commands::sysmon::sysfloat_set_visible,
            crate::commands::sysmon::sysfloat_metrics_get,
            crate::commands::sysmon::sysfloat_metrics_set,
            crate::commands::sysmon::sysfloat_pin_get,
            crate::commands::sysmon::sysfloat_pin_set,
            crate::commands::sysmon::sysfloat_shape_get,
            crate::commands::sysmon::sysfloat_expand,
            crate::commands::sysmon::sysfloat_fit,
            crate::commands::remote::remote_status,
            crate::commands::remote::remote_scan,
            crate::commands::remote::remote_connect,
            crate::commands::remote::remote_disconnect,
            crate::commands::remote::remote_buttons_get,
            crate::commands::remote::remote_panel_buttons_get,
            crate::commands::remote::remote_panel_buttons_set,
            crate::commands::remote::remote_recording_set,
            crate::commands::remote::remote_simulate_key,
            crate::commands::remote::remote_buttons_set,
            crate::commands::remote::remote_schemes_get,
            crate::commands::remote::remote_scheme_current,
            crate::commands::remote::remote_scheme_save,
            crate::commands::remote::remote_scheme_apply,
            crate::commands::remote::remote_scheme_rename,
            crate::commands::remote::remote_scheme_delete,
            crate::commands::remote::remote_scheme_export,
            crate::commands::remote::remote_scheme_import,
            crate::commands::remote::remote_front_app_get,
            crate::commands::remote::remote_app_bindings_get,
            crate::commands::remote::remote_app_binding_set,
            crate::commands::remote::remote_macros_get,
            crate::commands::remote::remote_macro_save,
            crate::commands::remote::remote_macro_delete,
            crate::commands::remote::remote_key_interaction_get,
            crate::commands::remote::remote_key_interaction_set,
            crate::commands::remote::remote_app_focus_get,
            crate::commands::remote::remote_app_focus_set,
            crate::commands::remote::remote_voice_get,
            crate::commands::remote::remote_voice_set,
            crate::commands::remote::remote_mouse_mode_get,
            crate::commands::remote::remote_mouse_mode_set,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_ime_list,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_ime_open_settings,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_perms_check,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_perm_open,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_apps_get,
            crate::commands::remote::remote_agents_cli,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_app_icon,
            #[cfg(target_os = "macos")]
            crate::commands::remote::remote_app_pick
        ])
        .manage(std::sync::Arc::new(
            crate::services::remote::identity::RemoteSession::new(),
        ))
        .manage(std::sync::Arc::new(
            crate::services::remote::voice::VoiceChannel::new(),
        ))
        .manage(std::sync::Arc::new(
            crate::services::remote::hid::ButtonChannel::new(),
        ))
        .setup(move |app| {
            // DEBUG ONLY：debug 构建自动开 Web Inspector，用于定位虚拟键盘卡死（JS 更新循环）。
            // 正式包不含此代码（cfg(debug_assertions)），不影响发布。
            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                w.open_devtools();
            }
            // 主窗口尺寸：window-state 插件在 hide-to-tray 下永不保存（且 macOS inner_size
            // 有上游 bug），尺寸由 save_main_size 在 hide/quit 时存 settings，这里读回。
            // setup 就是可靠时机：窗口已按 conf 建好、事件循环还没起，此时改尺寸看不到跳变。
            // 旧笔记那两个顾虑都已排除——panic 真因是 state() 未 manage，白屏真因是 vite 挂了。
            restore_main_size(app.handle(), &state.store);
            // UI-17：读回语言偏好（"zh"/"en"），重启后托盘菜单保持用户选的语言。
            if let Ok(Some(v)) = state.store.get_setting("ui.lang") {
                i18n::set_override(match v.as_str() {
                    "zh" => Some(i18n::Lang::Zh),
                    "en" => Some(i18n::Lang::En),
                    _ => None,
                });
            }
            // The gateway runs on tauri's own multi-thread runtime.
            let gw = state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = gateway::serve_forever(gw).await {
                    tracing::error!(target: "llm_switch::gateway", error = %e, "gateway exited");
                }
            });

            // S4 按键通道：起巡检认领遥控器 HID 接口（macOS 实现；stub 平台空转报不支持）。
            // 语音通道：偏好回读 + 事件出口（mic 键按下时由按键通道转交过来）。
            app.state::<std::sync::Arc<crate::services::remote::voice::VoiceChannel>>()
                .start(app.handle().clone(), state.clone(), &state.store);
            app.state::<std::sync::Arc<crate::services::remote::hid::ButtonChannel>>()
                .start(
                    app.handle().clone(),
                    state.store.clone(),
                    state.config.data_dir(),
                    app.state::<std::sync::Arc<crate::services::remote::voice::VoiceChannel>>()
                        .inner()
                        .clone(),
                    app.state::<std::sync::Arc<
                        crate::services::remote::identity::RemoteSession,
                    >>()
                        .inner()
                        .clone(),
                );
            // 演示驱动口（仅 debug）：`curl '127.0.0.1:8898/simulate?id=ok&gesture=click'`
            // 就能喂键，走与真键同一套事件出口——录片时由脚本驱动，不必真人按。
            #[cfg(debug_assertions)]
            crate::services::remote::hid::spawn_sim_server(
                app.state::<std::sync::Arc<crate::services::remote::hid::ButtonChannel>>()
                    .inner()
                    .clone(),
            );
            // 电量轮询：从系统已连接设备读电池服务（SayAll 同款：已连接不广播，扫描看不见）。
            let battery_session = app
                .state::<std::sync::Arc<crate::services::remote::identity::RemoteSession>>()
                .inner()
                .clone();
            tauri::async_runtime::spawn(crate::services::remote::battery::poll_loop(
                battery_session,
            ));

            // 代理扫描预热：启动即在后台并行探一轮（装没装、版本），填好缓存后
            // emit `agents://scan`。代理页首帧从此只读快照，不再被 8 个登录 shell 卡住。
            {
                use tauri::Emitter;
                let warm = app.handle().clone();
                crate::services::agents_scan::schedule_refresh(&crate::takeover::ClientApp::ALL, move || {
                    let _ = warm.emit("agents://scan", ());
                });
            }
            // 应用清单预热（用户 2026-11-23 拍板「启动就缓存好，用时无感」）：后台扫一轮
            // 已安装应用填好缓存，「打开应用」选择器打开时只读缓存、不必等扫盘。
            // 错开 2s 让窗口先出来；扫盘走 blocking 线程（同步命令会卡主线程，
            // 见 commands::remote::remote_apps_get 的口径注释）。
            #[cfg(target_os = "macos")]
            tauri::async_runtime::spawn(async {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if let Err(e) =
                    tauri::async_runtime::spawn_blocking(crate::services::remote::applist::warm).await
                {
                    tracing::warn!(target: "llm_switch::remote", error = %e, "应用清单预热失败");
                }
            });
            // 速率线：上次开着哪几家就点回哪几家，各自回各自的位置。
            rateline::restore(app.handle());
            // 资源小窗：上次开着就恢复（偏好驱动，见 gui/sysfloat.rs）。
            sysfloat::restore(app.handle());
            // 提示词速取面板：注册唤起快捷键（库存键坏了退回默认键）。
            promptfloat::register_hotkey(app.handle());

            // #46 代理有请求自动弹速率线：装网关→GUI 通道 + 监听循环。网关挂号
            // 在转发热路径，这里只做轻活：预检（自动开关/已开着）后换主线程建窗，
            // 三道闸的最终裁决在 rateline::auto_open（与预检同口径，双保险）。
            let (hit_tx, mut hit_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            if let Ok(mut slot) = state.agent_hit_tx.lock() {
                *slot = Some(hit_tx);
            }
            let hit_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(name) = hit_rx.recv().await {
                    // 预检只碰 settings（auto/master），窗口可见性留给主线程的
                    // auto_open——is_visible 走 AppKit，别在网关线程上碰窗口。
                    if !rateline::auto_on(&hit_app) || !rateline::master_on(&hit_app) {
                        continue;
                    }
                    let h = hit_app.clone();
                    // 建窗/置顶要在主线程（与 IPC 命令同线程口径）。
                    if let Err(e) = hit_app.run_on_main_thread(move || {
                        rateline::auto_open(&h, &name);
                    }) {
                        tracing::warn!(target: "llm_switch::rateline", error = %e, "自动弹速率线投递失败");
                    }
                }
            });

            // Monitor 推送：每秒快照活跃流，有变化或有活跃流才发（~1s 节流）。
            let mon_state = state.clone();
            let mon_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                let mut last_len = usize::MAX;
                // 自动浮窗 watcher 状态（切片 11）：只收自动开的窗，用户手动
                // 关就尊重到本轮流量结束；空闲 AUTO_IDLE 秒即收。
                let mut auto_shown = false;
                let mut suppressed = false;
                let mut idle_since: Option<std::time::Instant> = None;
                const AUTO_IDLE: std::time::Duration = std::time::Duration::from_secs(15);
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    // 出口形状与浮条完全一致（rates 模块），两处消费者不吃两套字段。
                    let rows = crate::gateway::rates::inflight_json(&mon_state);
                    let len = rows.len();
                    // 速率线空闲自动收起：在飞名单递给主线程巡检（is_visible 走
                    // AppKit，别在网关线程上碰窗口）；自动/总开关关着时不投递。
                    if rateline::auto_on(&mon_app) && rateline::master_on(&mon_app) {
                        let busy: std::collections::HashSet<String> = rows
                            .iter()
                            .filter_map(|r| {
                                r.get("app").and_then(|v| v.as_str()).map(str::to_string)
                            })
                            .collect();
                        let rl_app = mon_app.clone();
                        if let Err(e) = mon_app.run_on_main_thread(move || {
                            rateline::idle_sweep(&rl_app, &busy);
                        }) {
                            tracing::warn!(target: "llm_switch::rateline", error = %e, "速率线空闲巡检投递失败");
                        }
                    }
                    // 「收到 API 流量自动弹出浮动监视器」：开关每秒读一次（即时生效），
                    // 关闭时不碰窗——用户自己的窗用户自己管。
                    let auto_on = mon_state
                        .store
                        .get_setting("monitor.auto_float")
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some("true");
                    if !auto_on {
                        auto_shown = false;
                        suppressed = false;
                        idle_since = None;
                    } else if len > 0 {
                        idle_since = None;
                        if auto_shown && !floatwin::is_visible(&mon_app) {
                            // 自动开的窗被用户手动关了：本轮不再烦他。
                            auto_shown = false;
                            suppressed = true;
                        } else if !auto_shown && !suppressed && !floatwin::is_visible(&mon_app) {
                            floatwin::set_visible(&mon_app, true);
                            auto_shown = true;
                        }
                    } else if auto_shown {
                        let at = *idle_since.get_or_insert_with(std::time::Instant::now);
                        if at.elapsed() >= AUTO_IDLE {
                            floatwin::set_visible(&mon_app, false);
                            auto_shown = false;
                            idle_since = None;
                        }
                    } else {
                        // 流量收尾：清掉手动关标记，下一轮流量可以再自动开。
                        suppressed = false;
                    }
                    if len == 0 && last_len == 0 {
                        // 空闲时不发流表，但速率卡仍需归零心跳。
                        let snap = mon_state.metrics.snapshot(0);
                        let _ = mon_app.emit("metrics://tick", snap);
                        continue;
                    }
                    last_len = len;
                    let _ = mon_app.emit("monitor://streams", rows);
                    let snap = mon_state.metrics.snapshot(len);
                    let _ = mon_app.emit("metrics://tick", snap);
                }
            });

            // 日志 tail 推流：订阅 sink，1s 合并批量下发。
            let tail_state = state.clone();
            let tail_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                let mut rx = tail_state.log.subscribe_tail();
                loop {
                    let mut batch: Vec<_> = Vec::new();
                    // 阻塞取第一条（限错误跳出），随后 1s 内的都并进批次。
                    match rx.recv().await {
                        Ok(row) => batch.push(row),
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(_) => continue,
                    }
                    let deadline = tokio::time::sleep(std::time::Duration::from_millis(1000));
                    tokio::pin!(deadline);
                    loop {
                        tokio::select! {
                            _ = deadline.as_mut() => break,
                            r = rx.recv() => match r {
                                Ok(row) => batch.push(row),
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                Err(_) => break,
                            },
                        }
                    }
                    let _ = tail_app.emit("logs://tail", batch);
                }
            });

            let menu = build_menu(app.handle(), &state)?;
            let tray_state = state.clone();
            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .menu(&menu)
                .tooltip("llm-switch")
                .on_menu_event(move |app, event| handle_menu(app, &tray_state, event.id.as_ref()))
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        focus_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // 托盘菜单 60s 刷一次：今日用量在走，暂停态也可能被别处（REST/UI）改。
            {
                let tick_app = app.handle().clone();
                let tick_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        refresh_tray(&tick_app, &tick_state);
                    }
                });
            }
            if float_at_start {
                // 稍等一下再拉：等网关监听就绪、webview 挂好轮询，免得首帧空板。
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    tracing::info!(target: "llm_switch::float", "--float 定时到点，准备显示浮窗");
                    floatwin::set_visible(&handle, true);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 浮窗移动中要做贴边判定（每帧检查是否与屏幕边相接；相接才缩圆钮 / 贴合）。
            if window.label() == floatwin::LABEL {
                if let tauri::WindowEvent::Moved(_) = event {
                    floatwin::on_moved(&window.app_handle().clone());
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 主窗口点关闭=hide-to-tray，插件不会存状态——这里补存真实尺寸，
                // 下次启动读回（托盘退出那条路径在 handle_menu 的 quit 里另存）。
                if window.label() == "main" {
                    // 用 try_state（安全返回 Option），绝不用 state()——后者在未 manage 时
                    // 会 non-unwinding panic 直接 abort 进程。
                    if let Some(st) = window.app_handle().try_state::<State>() {
                        save_main_size(window.app_handle(), &st.store);
                    }
                }
                // Hide to tray instead of tearing the gateway down.
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())?;

    app.run(|app, event| {
        match event {
            // macOS 点 Dock 图标（或 `open -a`）在窗口被隐藏后会来 Reopen。
            // 关闭窗口=隐藏到托盘，此前无人接这个事件 → Dock 点击毫无反应
            // （用户实报）。统一唤回主窗口（隐藏/最小化/失焦全盖住）。
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows,
                ..
            } => {
                tracing::debug!(
                    target: "llm_switch::gui",
                    "dock reopen (has_visible_windows={has_visible_windows}) → focus_main"
                );
                focus_main(app);
            }
            // Keep the gateway alive when the last window is hidden (close hides,
            // so exiting only ever happens via the tray menu or exit(0)).
            RunEvent::ExitRequested { .. } => {}
            RunEvent::Exit => {
                // 退出兜底：按住态（hold 动作）没落定就退出，会把 ⌘ 卡在系统里。
                // inject 是 macos+gui 门（Linux 无 hold 注入路径），cfg 必须跟着走。
                #[cfg(all(feature = "gui", target_os = "macos"))]
                crate::services::remote::inject::release_held_mods();
            }
            _ => {}
        }
    });
    Ok(())
}

/// 托盘 id（菜单刷新时要拿回这个托盘）。
const TRAY_ID: &str = "main-tray";

/// 今日（UTC 日界，与 usage_daily 同口径）用量摘要。
fn today_summary(state: &State) -> String {
    let now = crate::types::now_ms();
    let day_start = now / 86_400_000 * 86_400_000;
    let summary = state.store.usage_summary(day_start, now).ok();
    let (requests, tokens) = match &summary {
        Some(s) => (
            s.requests,
            s.input_tokens + s.output_tokens + s.cache_read_tokens,
        ),
        None => (0, 0),
    };
    let cost = crate::services::pricing::usage_cost(&state.store, day_start, now)
        .ok()
        .and_then(|v| v["total_cost_usd"].as_f64())
        .unwrap_or(0.0);
    i18n::lang().stats(requests, &human_i64(tokens), cost)
}

fn human_i64(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// 托盘菜单：今日用量 / 快速启停 / 快速切后端 / 窗口。
/// 文案一律取自 `i18n`（禁止在此写死单语言字面量——曾有 "Show Window" /
/// "Quit llm-switch" 写死英文，中文环境照样出英文的实报问题）。
fn build_menu(app: &AppHandle, state: &State) -> tauri::Result<Menu<tauri::Wry>> {
    let l = i18n::lang();
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "stats",
        today_summary(state),
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let toggle_label = if state.is_paused() {
        l.resume()
    } else {
        l.pause()
    };
    menu.append(&MenuItem::with_id(
        app,
        "toggle-pause",
        toggle_label,
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let switch = Submenu::with_id(app, "switch", l.quick_switch(), true)?;
    let backends = state.store.list_backends().unwrap_or_default();
    let enabled = backends.iter().filter(|b| b.enabled).count();
    for b in &backends {
        let mark = if b.enabled { "\u{2713} " } else { "" };
        switch.append(&MenuItem::with_id(
            app,
            format!("be:{}", b.name),
            format!("{mark}{}", b.name),
            true,
            None::<&str>,
        )?)?;
    }
    if backends.is_empty() {
        switch.append(&MenuItem::with_id(
            app,
            "no-backends",
            l.no_backends(),
            false,
            None::<&str>,
        )?)?;
    }
    switch.append(&PredefinedMenuItem::separator(app)?)?;
    let all_mark = if !backends.is_empty() && enabled == backends.len() {
        "\u{2713} "
    } else {
        ""
    };
    switch.append(&MenuItem::with_id(
        app,
        "be-all",
        format!("{all_mark}{}", l.enable_all()),
        true,
        None::<&str>,
    )?)?;
    menu.append(&switch)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "float-toggle",
        l.float_toggle(),
        true,
        None::<&str>,
    )?)?;
    // 资源小窗（硬件监控浮窗）显隐：勾标取偏好（窗口可能还没建，偏好即实况，
    // 与设置页开关同源）。
    let sys_mark = if sysfloat::visible_pref(app) {
        "\u{2713} "
    } else {
        ""
    };
    menu.append(&MenuItem::with_id(
        app,
        "sysfloat-toggle",
        format!("{sys_mark}{}", l.sysfloat_toggle()),
        true,
        None::<&str>,
    )?)?;
    // 提示词速取面板：唤起全靠全局快捷键，托盘是快捷键被占用时的兜底入口。
    // 不带勾标——面板没有「常开」态（弹出→取用→收起），勾标永远是错的。
    menu.append(&MenuItem::with_id(
        app,
        "prompt-toggle",
        l.prompt_toggle(),
        true,
        None::<&str>,
    )?)?;
    // 速率线二级菜单：第一项是总开关（勾上 = 代理页勾着的那些都放着，取消 = 全部
    // 收起但启用清单不动），下面逐家单独开关——从前只有整批显隐，想只留一家还得回
    // 代理页点开。名单取 [`crate::gateway::origin::APPS`]：那是「网关能从 socket 认出
    // 是谁在发请求」的口径，认不出的代理开了线也只能一直空闲。
    // 标题不挂勾（它是容器）；第一项是动作项「全部显示/全部隐藏」；逐家的勾一律取
    // [`rateline::is_open`]——此刻真的看得见才打勾。只看启用清单会勾着一排却什么都
    // 看不见（总开关收起时清单一个字不动），用户点一下反而像没反应。
    let master = rateline::master_on(app);
    let lines = Submenu::with_id(app, "rateline", l.rate_lines(), true)?;
    lines.append(&MenuItem::with_id(
        app,
        "rateline-toggle",
        if master {
            l.rate_lines_hide_all()
        } else {
            l.rate_lines_show_all()
        },
        true,
        None::<&str>,
    )?)?;
    // 自动弹出开关（#46）：勾上 = 代理一有请求就点亮它家的线；总开关收起时不越权。
    let auto = rateline::auto_on(app);
    lines.append(&MenuItem::with_id(
        app,
        "rateline-auto",
        format!(
            "{}{}",
            if auto { "\u{2713} " } else { "" },
            l.rate_lines_auto()
        ),
        true,
        None::<&str>,
    )?)?;
    lines.append(&PredefinedMenuItem::separator(app)?)?;
    for name in crate::gateway::origin::APPS.iter() {
        let mark = if rateline::is_open(app, name) {
            "\u{2713} "
        } else {
            ""
        };
        lines.append(&MenuItem::with_id(
            app,
            format!("rateline:{name}"),
            format!("{mark}{name}"),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&lines)?;
    // 遥控器（macOS 独有：HID 按键/语音通道）：语音转写路快捷切换，勾标取当前偏好。
    // 与语音卡同一份设置（VoiceChannel.set_settings 落库并即时生效）；网关路没选
    // 模型时 set_settings 会拒——勾标不动就是被拒的可见反馈。
    #[cfg(target_os = "macos")]
    {
        let cur = app
            .try_state::<Arc<crate::services::remote::voice::VoiceChannel>>()
            .and_then(|ch| {
                ch.snapshot()["settings"]["route"]
                    .as_str()
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let remote = Submenu::with_id(app, "remote", l.remote(), true)?;
        for route in ["system", "soundcard", "gateway", "off"] {
            let mark = if cur == route { "\u{2713} " } else { "" };
            remote.append(&MenuItem::with_id(
                app,
                format!("voice:{route}"),
                format!("{mark}{}", l.voice_route(route)),
                true,
                None::<&str>,
            )?)?;
        }
        menu.append(&remote)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show",
        l.show_window(),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        l.quit(),
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn refresh_tray(app: &AppHandle, state: &std::sync::Arc<State>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_menu(app, state) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// 命令侧的托盘重建入口（UI-17：语言偏好写入后立刻换菜单文案）。
pub fn refresh_tray_now(app: &AppHandle, state: &std::sync::Arc<State>) {
    refresh_tray(app, state);
}

/// 托盘菜单动作。
fn handle_menu(app: &AppHandle, state: &std::sync::Arc<State>, id: &str) {
    match id {
        "quit" => {
            // app.exit(0) 不触发窗口 CloseRequested——退出前补存主窗口尺寸，
            // 否则「调好大小→托盘退出」这条最常见路径会丢。
            save_main_size(app, &state.store);
            app.exit(0);
        }
        "show" => focus_main(app),
        "float-toggle" => {
            floatwin::toggle(app);
        }
        "sysfloat-toggle" => {
            sysfloat::toggle(app);
            // 勾标要跟着翻，否则菜单看着永远是对的那一项。
            refresh_tray(app, state);
        }
        "prompt-toggle" => {
            promptfloat::toggle(app);
        }
        "rateline-toggle" => {
            rateline::set_master(app, !rateline::master_on(app));
            // 勾标要跟着翻，否则菜单看着永远是对的那一项。
            refresh_tray(app, state);
        }
        "rateline-auto" => {
            rateline::set_auto(app, !rateline::auto_on(app));
            refresh_tray(app, state);
        }
        "toggle-pause" => {
            let paused = !state.is_paused();
            if state.set_paused(paused) {
                let (code, message) = if paused {
                    (
                        "GW410 gateway.paused",
                        "gateway stopped accepting inference traffic",
                    )
                } else {
                    (
                        "GW411 gateway.resumed",
                        "gateway resumed accepting inference traffic",
                    )
                };
                state.log.log(crate::gateway::log::Event {
                    level: crate::gateway::log::Level::Info,
                    code,
                    message,
                    request_id: None,
                    backend: None,
                    model: None,
                    extra: serde_json::json!({ "via": "tray" }),
                });
            }
            refresh_tray(app, state);
        }
        "be-all" => {
            let _ = crate::services::backend::apply_backend_selection(&state.store, None);
            refresh_tray(app, state);
        }
        other => {
            if let Some(target) = other.strip_prefix("rateline:") {
                // 逐家开关：开着就收，关着就放（open 自己会把总开关一起点亮）。
                if rateline::is_open(app, target) {
                    rateline::close(app, target);
                } else {
                    rateline::open(app, target);
                }
                refresh_tray(app, state);
            } else if let Some(name) = other.strip_prefix("be:") {
                let _ = crate::services::backend::apply_backend_selection(&state.store, Some(name));
                refresh_tray(app, state);
            } else if let Some(route) = other.strip_prefix("voice:") {
                // 读当前整份偏好、只改 route 再写回：set_settings 吃整份 VoiceSettings，
                // 裸 {"route":…} 会把设备/音量/输入法按 serde 缺省抹掉。
                #[cfg(target_os = "macos")]
                if let Some(ch) =
                    app.try_state::<Arc<crate::services::remote::voice::VoiceChannel>>()
                {
                    let mut settings = ch.snapshot()["settings"].clone();
                    settings["route"] = serde_json::Value::String(route.to_string());
                    let _ = ch.set_settings(&state.store, settings);
                }
                refresh_tray(app, state);
            }
        }
    }
}

fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

// ---- 主窗口尺寸持久化 -------------------------------------------------------
// 为什么不靠 window-state 插件：① 关闭窗口=hide-to-tray，主窗口从不真正 Close，
// 插件的保存时机永远不来；② macOS 上插件用 inner_size() 读到 conf 固定值（上游
// issue #926 至今未修），存下来的也是错的。这里自己用 settings 存 outer_size()
// （真实尺寸），在 hide / 托盘退出两处落盘，setup 时由 restore_main_size 读回。

/// 主窗口尺寸的 settings key（`宽,高`，物理像素，outer 口径）。
const MAIN_SIZE_KEY: &str = "main_window.size";

/// 保存主窗口当前尺寸（outer_size = 含标题栏的真实尺寸）。
/// store 由调用方传入——绝不用 `app.state::<State>()`：setup 早期 State 还没 manage，
/// 那个 API 会 panic 且是 non-unwinding，整个进程 abort（实锤踩过）。
fn save_main_size(app: &AppHandle, store: &Store) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let Ok(size) = win.outer_size() else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return;
    }
    if let Err(e) = store.set_setting(MAIN_SIZE_KEY, &format!("{},{}", size.width, size.height)) {
        tracing::warn!(target: "llm_switch::gui", error = %e, "主窗口尺寸写入失败");
    }
}

/// 把 settings 存的 outer 尺寸换算成 `set_size` 要的 inner 口径（减掉标题栏/边框）。
/// 存 outer 是因为 macOS 上 `inner_size()` 不可信（上游 #926，window-state 插件就栽在这），
/// 而 `set_size` 设的是 inner：直接拿 outer 当 inner 用，每次启动会长高一截标题栏。
/// `deco` 是窗口当前的 outer-inner；启动时窗口刚按 conf 建好，这个差值恒等于装饰高。
/// shortcut: 存物理像素，换不同缩放的外接屏后尺寸可能偏大，用户拖一下即可，未做跨屏换算。
fn main_inner_size(raw: &str, deco: (u32, u32)) -> Option<(u32, u32)> {
    let mut it = raw.split(',');
    let w = it.next()?.trim().parse::<u32>().ok()?;
    let h = it.next()?.trim().parse::<u32>().ok()?;
    if it.next().is_some() {
        return None;
    }
    let (w, h) = (w.saturating_sub(deco.0), h.saturating_sub(deco.1));
    (w > 0 && h > 0).then_some((w, h))
}

/// 启动时读回主窗口尺寸。store 由调用方传入——绝不用 `app.state::<State>()`：setup 早期
/// State 还没 manage，那个 API 是 non-unwinding panic，整个进程 abort（实锤踩过）。
fn restore_main_size(app: &AppHandle, store: &Store) {
    let Ok(Some(raw)) = store.get_setting(MAIN_SIZE_KEY) else {
        return;
    };
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    // 读不到就按无装饰算；两个 getter 在主线程走同步快路径，setup 里调不会卡死。
    let deco = match (win.outer_size(), win.inner_size()) {
        (Ok(o), Ok(i)) => (
            o.width.saturating_sub(i.width),
            o.height.saturating_sub(i.height),
        ),
        _ => (0, 0),
    };
    let Some((w, h)) = main_inner_size(&raw, deco) else {
        tracing::warn!(target: "llm_switch::gui", raw, "主窗口尺寸读数无法解析，忽略");
        return;
    };
    let _ = win.set_size(PhysicalSize::new(w, h));
}

#[cfg(test)]
mod tests {
    use super::main_inner_size;

    #[test]
    fn 读回时减掉标题栏高度() {
        // 存 outer 1400x900、标题栏 28 → 要设 inner 1400x872，否则每次启动长高一截
        assert_eq!(main_inner_size("1400,900", (0, 28)), Some((1400, 872)));
        assert_eq!(main_inner_size(" 1400 , 900 ", (0, 0)), Some((1400, 900)));
    }

    #[test]
    fn 坏读数与减完为零的一律忽略() {
        for raw in ["", "1400", "abc,900", "1400,900,7", "0,900", "-1,900"] {
            assert_eq!(main_inner_size(raw, (0, 0)), None, "{raw}");
        }
        // 存的比装饰还小 = 脏数据，别把窗口压成零
        assert_eq!(main_inner_size("1400,20", (0, 28)), None);
    }
}

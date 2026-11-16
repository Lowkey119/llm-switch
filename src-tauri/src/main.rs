use llm_switch::config::Config;
use llm_switch::gateway::{self, run};
use llm_switch::store::Store;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Default)]
struct Args {
    config: Option<PathBuf>,
    port: Option<u16>,
    data_dir: Option<PathBuf>,
    gui: bool,
    headless: bool,
    float: bool,
    swap: bool,
    help: bool,
    version: bool,
}

impl Args {
    fn parse<I: Iterator<Item = String>>(mut iter: I) -> Result<Self, String> {
        let mut out = Args::default();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-c" | "--config" => {
                    let v = iter
                        .next()
                        .ok_or_else(|| "--config needs a value".to_string())?;
                    out.config = Some(PathBuf::from(v));
                }
                "-p" | "--port" => {
                    let v = iter
                        .next()
                        .ok_or_else(|| "--port needs a value".to_string())?;
                    out.port = Some(v.parse().map_err(|_| format!("bad port `{v}`"))?);
                }
                "-d" | "--data-dir" => {
                    let v = iter
                        .next()
                        .ok_or_else(|| "--data-dir needs a value".to_string())?;
                    out.data_dir = Some(PathBuf::from(v));
                }
                "-h" | "--help" => out.help = true,
                "--gui" => out.gui = true,
                "--swap" => out.swap = true,
                "--headless" => out.headless = true,
                "--float" => out.float = true,
                "-V" | "--version" => out.version = true,
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        Ok(out)
    }
}

const HELP: &str = "\
llm-switch — local gateway in front of many LLM backends

USAGE:
    llm-switch [OPTIONS]

OPTIONS:
    -c, --config <PATH>      config file (default ~/.llm-switch/config.yaml)
    -p, --port <PORT>        override listen port
    --swap                   listen on server.swap_port and allow a parallel
                             instance (seamless upgrade: start new beside the
                             old one, verify, then retire the old)
    -d, --data-dir <PATH>    override data directory (db, logs, cache)
        --gui                force the desktop shell (requires --features gui)
        --headless           force headless mode even inside a .app bundle
        --float              show the floating rate panel at startup (desktop shell only)
    -h, --help               print this help
    -V, --version            print version
";

/// 是否从 macOS `.app` 包内被拉起（Finder 双击不会传 `--gui`，
/// 那样就只有 Dock 图标没有界面，所以包内启动默认走桌面壳）。
fn launched_from_app_bundle() -> bool {
    std::env::current_exe()
        .map(|p| p.display().to_string().contains(".app/Contents/MacOS/"))
        .unwrap_or(false)
}

/// 桌面壳还是无界面，抽成纯函数好测。
fn wants_gui(requested: bool, headless: bool, in_bundle: bool, feature_gui: bool) -> bool {
    if headless {
        return false;
    }
    requested || (feature_gui && in_bundle)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse(std::env::args().skip(1)).unwrap_or_else(|err| {
        eprintln!("llm-switch: {err}\n\n{HELP}");
        std::process::exit(2);
    });

    if args.help {
        println!("{HELP}");
        return Ok(());
    }
    if args.version {
        println!("llm-switch {}", llm_switch::VERSION);
        return Ok(());
    }

    let config_path = args
        .config
        .clone()
        .unwrap_or_else(|| dirs_home().join(".llm-switch").join("config.yaml"));
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        eprintln!(
            "llm-switch: no config at {}, starting with defaults (no backends)",
            config_path.display()
        );
        Config::default()
    };
    if let Some(port) = args.port {
        config.server.port = port;
    }
    // 换血模式：听 swap_port，与旧实例并行共存（单实例锁放行，见 gui::run）。
    if args.swap {
        config.server.port = config.server.swap_port;
    }
    if let Some(dir) = args.data_dir {
        config.data_dir = Some(dir);
    }
    config.validate()?;

    llm_switch::gateway::loglevel::init(&config.logs.level);

    let store = Arc::new(Store::open(&config.db_path())?);
    // settings 表里的持久级别优先于 yaml（上次运行中被 UI 改过）；RUST_LOG 最高。
    if std::env::var_os("RUST_LOG").is_none() {
        if let Ok(Some(lvl)) = store.get_setting("logs.level") {
            let _ = llm_switch::gateway::loglevel::set_level(&lvl);
        }
    }
    let config = Arc::new(config);
    let seeded = gateway::seed_backends(&store, &config)?;

    // 受管端的接管配置在启动时也对一次表：无差异不写盘、写之前照例留备份。
    // 从前只在「后端刷新 / 改模型真值」时重写，于是装了新版本重启完，清单里该隐的
    // 还在那儿——用户看到的是「改了个寂寞」。口径与别处的 resync 完全一致。
    let takeover_env = llm_switch::takeover::TakeoverEnv::from_config(&config, &store);
    let resynced = llm_switch::takeover::resync_managed(&takeover_env, &store, &config);
    if !resynced.is_empty() {
        tracing::info!(
            target: "llm_switch::takeover",
            count = resynced.len(),
            "startup resync rewrote managed takeover configs"
        );
    }

    // 速率扩展的取数入口：各家代理的扩展跑在自己进程里，拿不到运行时的端口与令牌，
    // 这里落一份只读快照（胶囊改端口时同步重写，见 `commands::panel::set_listen_port`）。
    // 端口必须走 `effective_listen_port`：胶囊改过端口后 settings 表才是真值，
    // 直接取 config.server.port 会在重启时把发现文件写回 yaml 旧端口，扩展集体断供。
    let listen_port = gateway::effective_listen_port(&store, &config);
    if let Err(e) = llm_switch::services::ext_install::write_endpoint_file_for_app(
        &config.data_dir(),
        &config.server.host,
        listen_port,
        config.server.access_token.as_deref(),
    ) {
        eprintln!("llm-switch: 速率端点发现文件写入失败（代理侧速率扩展会取不到数）：{e}");
    }

    if wants_gui(
        args.gui,
        args.headless,
        launched_from_app_bundle(),
        cfg!(feature = "gui"),
    ) {
        #[cfg(feature = "gui")]
        return llm_switch::gui::run(store, config, args.float, args.swap || args.port.is_some());
        #[cfg(not(feature = "gui"))]
        {
            eprintln!("llm-switch: this binary was built without the desktop shell; rebuild with `cargo build --features gui`");
            std::process::exit(2);
        }
    }

    println!(
        "llm-switch {} · http://{}:{} · {} backend(s) from config · data {}",
        llm_switch::VERSION,
        config.server.host,
        listen_port,
        seeded,
        config.data_dir().display()
    );

    run(store, config).await
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_and_rejects_junk() {
        let a = Args::parse(
            [
                "--config",
                "/tmp/a.yaml",
                "--port",
                "9100",
                "--data-dir",
                "/tmp/d",
            ]
            .iter()
            .map(|s| s.to_string()),
        )
        .unwrap();
        assert_eq!(a.port, Some(9100));
        assert_eq!(a.config.unwrap(), PathBuf::from("/tmp/a.yaml"));
        assert!(Args::parse(["--nope"].iter().map(|s| s.to_string())).is_err());
        assert!(Args::parse(["--port"].iter().map(|s| s.to_string())).is_err());
        assert!(Args::parse(["--port", "abc"].iter().map(|s| s.to_string())).is_err());
        assert!(
            Args::parse(["--headless"].iter().map(|s| s.to_string()))
                .unwrap()
                .headless
        );
        // --float：启动即弹浮窗（只在桌面壳里有意义，解析层面先收下）
        assert!(
            Args::parse(["--float"].iter().map(|s| s.to_string()))
                .unwrap()
                .float
        );
        assert!(!Args::parse(Vec::<String>::new().into_iter()).unwrap().float);
    }

    #[test]
    fn app_bundle_defaults_to_the_desktop_shell() {
        // 包内双击：没参数也进桌面壳
        assert!(wants_gui(false, false, true, true));
        // 裸二进制：保持无界面，除非显式 --gui
        assert!(!wants_gui(false, false, false, true));
        assert!(wants_gui(true, false, false, true));
        // --headless 一票否决（包内也想当服务跑）
        assert!(!wants_gui(true, true, true, true));
        assert!(!wants_gui(false, false, true, false));
    }
}

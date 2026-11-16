//! 系统资源监控（主计划 §10 / 切片 12）：本机 macOS + 102（SSH）硬件指标。
//! 展示形态借鉴 unsloth studio 的监控界面——**只借鉴思路，未复制任何代码**（AGPL 红线）。
//!
//! 红线：拿不到的指标如实报 null（UI 显示 —），绝不编数。
//! - 本机温度/功率：macmon（MIT，Apple Silicon 免 sudo：IOReport + SMC）→
//!   CPU/GPU 均温 + CPU/GPU/ANE 合计功率（与 102 PPT 封装口径对齐）；拿不到如实 null。
//! - 本机 GPU 利用率：IOAccelerator PerformanceStatistics（ioreg，免 sudo 实测可读）；
//!   设备名 system_profiler 懒取一次永久缓存（~1.7s，只发生一次）。
//! - 102：一次 SSH 往返拿齐 sensors/free/loadavg/df/amdgpu sysfs（全部实测可读，
//!   含 GPU 利用率与显存——APU 划走的 8G 显存）。
//! - TTL 缓存：本机 2s、102 5s、存储分解 60s（du 慢）。

use std::path::Path;
use std::sync::Mutex;
// Arc 只在 Apple Silicon 分支里用到；非该平台时不收，免得空导入。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sysinfo::{Disks, System};

use crate::config::HalctlCfg;
use crate::services::halctl;

/// 本机快照 TTL（监控页 3s 轮询，多消费者共享缓存）。
const LOCAL_TTL: Duration = Duration::from_secs(2);
/// 102 快照 TTL：SSH 往返贵，5s 足够。
const REMOTE_TTL: Duration = Duration::from_secs(5);

/// 102 sensors 标签口径（实测核实：k10temp Tctl=CPU 温度、amdgpu edge=GPU 温度、
/// PPT=封装功率）。换机器这些标签可能变——提不到就是 null，UI 显示 —。
const SENS_CPU_TEMP: &str = "Tctl:";
const SENS_GPU_TEMP: &str = "edge:";
const SENS_POWER: &str = "PPT:";

/// 102 采集命令：一次往返全拿，section 标记分段解析（halctl::status 同款手法）。
const REMOTE_CMD: &str = "echo __HOST__; hostname; \
echo __SENS__; sensors 2>/dev/null; \
echo __MEM__; free -b | awk 'NR==2{print $2,$3,$7}'; \
echo __LOAD__; cat /proc/loadavg; echo; nproc; \
echo __DISK__; df -B1 / | awk 'NR==2{print $2,$4}'; \
echo __GPU__; for c in /sys/class/drm/card*/device; do [ -f $c/gpu_busy_percent ] \
&& echo busy=$(cat $c/gpu_busy_percent 2>/dev/null) vused=$(cat $c/mem_info_vram_used 2>/dev/null) \
vtotal=$(cat $c/mem_info_vram_total 2>/dev/null) && break; done; \
echo __UP__; cut -d. -f1 /proc/uptime";

/// 跑本机命令拿 stdout（与 halctl 同思路：命令拼装 → 文本解析）；失败 None。
async fn run_local(cmd: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 取 `__MARK__` 段（到下一个 `__` 标记或结尾）。
fn section(text: &str, mark: &str) -> String {
    let start = match text.find(mark) {
        Some(i) => i + mark.len(),
        None => return String::new(),
    };
    let rest = &text[start..];
    match rest.find("\n__") {
        Some(i) => rest[..i].to_string(),
        None => rest.to_string(),
    }
}

/// 从以 `key` 开头的行里抠第一个数值（"Tctl:  +70.4°C" → 70.4）。
fn parse_num_after(text: &str, key: &str) -> Option<f64> {
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key) {
            let num: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// IOAccelerator 性能统计：`"Device Utilization %"=7` → 7.0。
fn parse_ioreg_util(text: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\"=");
    for line in text.lines() {
        if let Some(i) = line.find(&pat) {
            let rest = &line[i + pat.len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// `key=value` 抠数值（102 sysfs 汇总行 `busy=82 vused=... vtotal=...`）。
fn parse_kv(text: &str, key: &str) -> Option<f64> {
    for word in text.split_whitespace() {
        if let Some(v) = word.strip_prefix(&format!("{key}=")) {
            if let Ok(n) = v.parse::<f64>() {
                return Some(n);
            }
        }
    }
    None
}

/// 102 采集输出 → 视图。缺段/缺值一律 null（如实 —）。
fn remote_view(stdout: &str) -> Value {
    let sens = section(stdout, "__SENS__");
    let mem: Vec<f64> = section(stdout, "__MEM__")
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    // section 以换行开头，先 trim 再按行取（load 行 + 核数行；中间可能有空行）。
    let load_txt = section(stdout, "__LOAD__");
    let mut load_lines = load_txt.trim().lines().filter(|l| !l.trim().is_empty());
    let load_secs: Vec<f64> = load_lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    let cores = load_lines.next().and_then(|s| s.trim().parse::<u32>().ok());
    let disk: Vec<f64> = section(stdout, "__DISK__")
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let gpu = section(stdout, "__GPU__");
    let host = section(stdout, "__HOST__").trim().to_string();
    let g = |v: Option<f64>| match v {
        Some(n) => json!(n),
        None => Value::Null,
    };
    json!({
        "ok": true,
        "hostname": if host.is_empty() { Value::Null } else { json!(host) },
        "cpu_temp": g(parse_num_after(&sens, SENS_CPU_TEMP)),
        "gpu_temp": g(parse_num_after(&sens, SENS_GPU_TEMP)),
        "power_w": g(parse_num_after(&sens, SENS_POWER)),
        "mem_total": g(mem.first().copied()),
        "mem_used": g(mem.get(1).copied()),
        "mem_available": g(mem.get(2).copied()),
        "load": if load_secs.len() == 3 { json!(load_secs) } else { Value::Null },
        "cpu_cores": g(cores.map(|c| c as f64)),
        "disk_total": g(disk.first().copied()),
        "disk_free": g(disk.get(1).copied()),
        "gpu_pct": g(parse_kv(&gpu, "busy")),
        "vram_used": g(parse_kv(&gpu, "vused")),
        "vram_total": g(parse_kv(&gpu, "vtotal")),
        "uptime_s": g(section(stdout, "__UP__").trim().parse::<f64>().ok()),
    })
}

/// 数据目录所在的盘：挂载点是数据目录前缀且最长者（macOS 上 /System/Volumes/Data
/// 比 / 更具体，正是容量所在的数据卷）；一条都不匹配时退回容量最大的盘。
fn root_disk(data_dir: &Path) -> Option<(f64, f64)> {
    let disks = Disks::new_with_refreshed_list().ok()?;
    let hit = disks
        .iter()
        .filter(|d| data_dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().to_string_lossy().len())
        .or_else(|| disks.iter().max_by_key(|d| d.total_space()));
    hit.map(|d| (d.total_space() as f64, d.available_space() as f64))
}

/// 单例（挂在 `State.sysmon`）：sysinfo 实例（CPU 差分窗口=轮询间隔）+ 各 TTL 缓存。
#[derive(Default)]
pub struct SysMon {
    inner: Mutex<Inner>,
}

/// Apple Silicon 后台传感线程（macmon，MIT，出处见 NOTICE-upstream.md）。
///
/// macmon 的 `Sampler` 持 IOReport 订阅裸指针，**不是 Send**——既不能挂进
/// `Mutex<Inner>` 随 `State` 移动，也不能跨线程共享。于是把采样圈死在专用
/// 线程里常驻（IOReport 差分基线随之稳定），只把读数快照经 `Arc<Mutex<…>>`
/// 渡给轮询方。线程内的失败一律退避重试，永不退出。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod sense {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// (CPU 温度, GPU 温度, 合计功率 W)；None = 尚未采出（读数方如实报 null）。
    pub type Shared = Arc<Mutex<Option<(Option<f64>, Option<f64>, Option<f64>)>>>;

    /// 相邻两次采样间隔（内部各带一个 500ms 差分窗）。
    const PERIOD: Duration = Duration::from_millis(1000);

    /// 拉起传感线程（框架要求 JoinHandle 可丢——drop 即 detach，进程内常驻）。
    pub fn start() -> Shared {
        let latest: Shared = Arc::new(Mutex::new(None));
        std::thread::Builder::new()
            .name("sysmon-sense".into())
            .spawn({
                let latest = Arc::clone(&latest);
                move || loop {
                    let mut s = match macmon::metrics::Sampler::new() {
                        Ok(s) => s,
                        Err(_) => {
                            // 初始化失败（异常环境）：退避后整线程重试。
                            std::thread::sleep(Duration::from_secs(3));
                            continue;
                        }
                    };
                    loop {
                        // 单轮读数失败不致命：下轮重来。
                        if let Ok(m) = s.get_metrics(500) {
                            *latest.lock().expect("sysmon sense lock") = Some((
                                m.temp.cpu_temp_avg.map(f64::from),
                                m.temp.gpu_temp_avg.map(f64::from),
                                Some(f64::from(m.all_power)),
                            ));
                        }
                        std::thread::sleep(PERIOD);
                    }
                }
            })
            .ok();
        latest
    }
}

#[derive(Default)]
struct Inner {
    /// sysinfo 初始化在 macOS 上不会失败，但失败也不该拖垮应用：None → 相关指标如实 null。
    sys: Option<System>,
    /// 首轮 CPU 占用没有差分基线 → null（sysinfo 的用法约束）。
    sampled: bool,
    local: Option<(Instant, Value)>,
    gpu_name: Option<String>,
    gpu_name_tried: bool,
    remote: Option<(Instant, Value)>,
    /// Apple Silicon 传感器共享读数（后台线程维护，见 sense 模块注）。
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    sense: Option<sense::Shared>,
}

impl SysMon {
    pub fn new() -> Self {
        Self::default()
    }

    /// 聚合快照 `{ local: {...}, remote: {...} }`（监控页「系统与引擎」卡）。
    pub async fn snapshot(&self, ctl: &HalctlCfg, data_dir: &Path) -> Value {
        let local = self.local_snapshot(data_dir).await;
        let remote = self.remote_snapshot(ctl).await;
        json!({ "local": local, "remote": remote })
    }

    async fn local_snapshot(&self, data_dir: &Path) -> Value {
        if let Some((at, v)) = self.inner.lock().expect("sysmon lock").local.as_ref() {
            if at.elapsed() < LOCAL_TTL {
                return v.clone();
            }
        }
        // sysinfo 部分纯进程内，锁短拿短放（锁守卫绝不跨 await）。
        // 拿不到的指标一律 null（编数是红线）：sysinfo 初始化失败、首轮无 CPU
        // 差分基线、load_average 出错，都如实报 null。
        let (cpu_pct, cores, load, mem_used, mem_total, mem_avail, disk) = {
            let mut guard = self.inner.lock().expect("sysmon lock");
            // 再借成 &mut Inner：MutexGuard 的 Deref 会把整个守卫借走，字段拆分失效。
            let g = &mut *guard;
            if g.sys.is_none() {
                g.sys = System::new().ok();
            }
            let (mut cpu, mut cores, mut load) = (None, None, None);
            let (mut m_used, mut m_total, mut m_avail) = (None, None, None);
            if let Some(sys) = g.sys.as_mut() {
                sys.refresh_cpu_usage();
                sys.refresh_memory();
                if g.sampled {
                    cpu = Some(sys.global_cpu_usage() as f64);
                } else {
                    g.sampled = true;
                }
                cores = Some(sys.cpus().len());
                load = System::load_average()
                    .ok()
                    .map(|la| [la.one, la.five, la.fifteen]);
                m_used = Some(sys.used_memory() as f64);
                m_total = Some(sys.total_memory() as f64);
                m_avail = Some(sys.available_memory() as f64);
            }
            (
                cpu,
                cores,
                load,
                m_used,
                m_total,
                m_avail,
                root_disk(data_dir),
            )
        };
        // macmon 读数（Apple Silicon）：Sampler 本体不可跨线程共享（非 Send，
        // 见 sense 模块注），这里只读后台线程渡出的快照；尚未采出时如实 null。
        let (m_cpu_temp, m_gpu_temp, m_power) = {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                let latest = {
                    let mut guard = self.inner.lock().expect("sysmon lock");
                    if let Some(s) = guard.sense.clone() {
                        s
                    } else {
                        let s = sense::start();
                        guard.sense = Some(Arc::clone(&s));
                        s
                    }
                };
                latest
                    .lock()
                    .ok()
                    .and_then(|l| *l)
                    .unwrap_or((None, None, None))
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                // 显式标出元组元素类型：macOS 分支被 cfg 关掉时，编译器无法从
                // 裸 (None, None, None) 推出 Option 的内层类型（E0282）。
                let idle: (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
                idle
            }
        };
        // GPU：ioreg 每 TTL 一次（免 sudo）；设备名懒取一次永久缓存。
        let gpu = run_local("ioreg", &["-r", "-c", "IOAccelerator"]).await;
        let (gpu_pct, gpu_renderer) = match &gpu {
            Some(t) => (
                parse_ioreg_util(t, "Device Utilization %"),
                parse_ioreg_util(t, "Renderer Utilization %"),
            ),
            None => (None, None),
        };
        let gpu_name = self.gpu_name().await;
        // 根盘读不到如实 null（塌成 0 也是编数）。
        let (disk_total, disk_free) = match disk {
            Some((t, f)) => (json!(t), json!(f)),
            None => (Value::Null, Value::Null),
        };
        let v = json!({
            "ok": true,
            "cpu_pct": cpu_pct,
            "cpu_cores": cores,
            "load": load,
            "mem_used": mem_used,
            "mem_total": mem_total,
            "mem_available": mem_avail,
            "gpu_name": gpu_name,
            "gpu_pct": gpu_pct,
            "gpu_renderer_pct": gpu_renderer,
            // Apple Silicon 显存与系统内存共享：VRAM 卡按「共享」呈现可用量。
            "vram_shared": true,
            // macmon 传感器（Apple Silicon）：拿不到/失败如实 null，UI 显示 —（编数是红线）。
            "cpu_temp": m_cpu_temp,
            "gpu_temp": m_gpu_temp,
            "power_w": m_power,
            "disk_total": disk_total,
            "disk_free": disk_free,
        });
        self.inner.lock().expect("sysmon lock").local = Some((Instant::now(), v.clone()));
        v
    }

    /// GPU 设备名：system_profiler 慢（~1.7s），只跑一次，之后永远用缓存。
    async fn gpu_name(&self) -> Option<String> {
        {
            let g = self.inner.lock().expect("sysmon lock");
            if g.gpu_name_tried {
                return g.gpu_name.clone();
            }
        }
        let name = run_local("system_profiler", &["SPDisplaysDataType"])
            .await
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.trim().strip_prefix("Chipset Model:"))
                    .map(|s| s.trim().to_string())
            });
        let mut g = self.inner.lock().expect("sysmon lock");
        g.gpu_name_tried = true;
        g.gpu_name = name.clone();
        name
    }

    async fn remote_snapshot(&self, ctl: &HalctlCfg) -> Value {
        if let Some((at, v)) = self.inner.lock().expect("sysmon lock").remote.as_ref() {
            if at.elapsed() < REMOTE_TTL {
                return v.clone();
            }
        }
        let v = match halctl::run(ctl, REMOTE_CMD, Duration::from_secs(20)).await {
            Ok(r) => remote_view(&r.stdout),
            Err(e) => json!({ "ok": false, "error": e }),
        };
        self.inner.lock().expect("sysmon lock").remote = Some((Instant::now(), v.clone()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_num_after_sensors_lines() {
        let sens = "k10temp-pci-00c3\nAdapter: PCI adapter\nTctl:         +70.4°C  \n\
                    amdgpu-pci-c500\nAdapter: PCI adapter\nedge:         +71.0°C  \nPPT:         107.02 W  (avg = 107.02 W)\n";
        assert_eq!(parse_num_after(sens, "Tctl:"), Some(70.4));
        assert_eq!(parse_num_after(sens, "edge:"), Some(71.0));
        assert_eq!(parse_num_after(sens, "PPT:"), Some(107.02));
        assert_eq!(parse_num_after(sens, "nope:"), None);
    }

    #[test]
    fn parse_ioreg_util_and_kv() {
        let ioreg = "  |   \"PerformanceStatistics\" = { \"Device Utilization %\"=7, \
                     \"Renderer Utilization %\"=6 }";
        assert_eq!(parse_ioreg_util(ioreg, "Device Utilization %"), Some(7.0));
        assert_eq!(parse_ioreg_util(ioreg, "Renderer Utilization %"), Some(6.0));
        assert_eq!(parse_ioreg_util(ioreg, "Missing %"), None);
        assert_eq!(
            parse_kv("busy=82 vused=1783 vtotal=8589", "vused"),
            Some(1783.0)
        );
        assert_eq!(parse_kv("busy=82", "vused"), None);
    }

    #[test]
    fn section_splits_on_next_mark() {
        let text = "x\n__A__\nline1\nline2\n__B__\nonly-b\n";
        assert_eq!(section(text, "__A__").trim(), "line1\nline2");
        assert_eq!(section(text, "__B__").trim(), "only-b");
        assert_eq!(section(text, "__C__"), "");
    }

    #[test]
    fn remote_view_parses_full_capture() {
        // 102 实测输出缩样（口径见 REMOTE_CMD：loadavg 与 nproc 间有空行）。
        let out = "__HOST__\nlowkey-AXB35-02\n__SENS__\nk10temp-pci-00c3\nTctl:         +70.4°C  \namdgpu-pci-c500\nedge:         +71.0°C  \nPPT:         107.02 W  (avg = 107.02 W)\n__MEM__\n123189469184 45312345600 80910000000\n__LOAD__\n1.12 1.64 1.86 2/1916 3577011\n\n32\n__DISK__\n316911181824 89876543210\n__GPU__\nbusy=82 vused=1783623680 vtotal=8589934592\n__UP__\n3180363\n";
        let v = remote_view(out);
        assert_eq!(v["ok"], true);
        assert_eq!(v["hostname"], "lowkey-AXB35-02");
        assert_eq!(v["cpu_temp"], 70.4);
        assert_eq!(v["gpu_temp"], 71.0);
        assert_eq!(v["power_w"], 107.02);
        assert_eq!(v["mem_total"], 123189469184.0);
        assert_eq!(v["cpu_cores"], 32.0);
        assert_eq!(v["load"][0], 1.12);
        assert_eq!(v["gpu_pct"], 82.0);
        assert_eq!(v["vram_total"], 8589934592.0);
        assert_eq!(v["uptime_s"], 3180363.0);
    }

    #[test]
    fn remote_view_missing_sections_are_null_not_zero() {
        // 编数是红线：缺段必须 null，不能塌成 0。
        let v = remote_view(
            "__SENS__\nnothing here\n__MEM__\n\n__LOAD__\n\n\n__DISK__\n\n__GPU__\n\n__UP__\n\n",
        );
        assert!(v["cpu_temp"].is_null());
        assert!(v["mem_total"].is_null());
        assert!(v["load"].is_null());
        assert!(v["gpu_pct"].is_null());
    }

    /// 活体冒烟（手动 `cargo test -- --ignored sysmon`）：真采本机 + 102，
    /// 打印快照供肉眼核对口径。不进常规门禁——依赖 SSH 免密。
    #[tokio::test]
    #[ignore]
    async fn live_snapshot_smoke() {
        let sm = SysMon::new();
        let data_dir = crate::config::Config::default().data_dir();
        let v = sm.snapshot(&HalctlCfg::default(), &data_dir).await;
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        assert_eq!(v["local"]["ok"], true);
        assert_eq!(v["remote"]["ok"], true, "102 SSH 采集失败：{v}");
        assert!(
            v["remote"]["cpu_cores"].is_number(),
            "102 核数应解析出来：{}",
            v["remote"]
        );
        // 根盘必须是真的数据卷（GB 级），不是 FileVault/Preboot 之类伪卷。
        assert!(
            v["local"]["disk_total"].as_f64().unwrap() > 1e9,
            "本机根盘异常：{}",
            v["local"]
        );
        // 第二轮（过了 LOCAL_TTL）：CPU 有差分基线后不再是 null。
        tokio::time::sleep(Duration::from_millis(2100)).await;
        let v2 = sm.local_snapshot(&data_dir).await;
        assert!(v2["cpu_pct"].is_number(), "第二轮 CPU 占用应有值：{v2}");
        // Apple Silicon 上 macmon 应给出真实温/功率（编数红线：拿不到就该报 null，
        // 但这台 M1 Max 实测有数——冒烟里断言非 null， Intel/其他平台自动跳过）。
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            println!(
                "本机传感: cpu_temp={} gpu_temp={} power_w={}",
                v2["cpu_temp"], v2["gpu_temp"], v2["power_w"]
            );
            assert!(
                v2["cpu_temp"].is_number() && v2["gpu_temp"].is_number(),
                "macmon 温度应有值：{v2}"
            );
            assert!(
                v2["power_w"].as_f64().is_some_and(|w| w > 0.0),
                "macmon 合计功率应 > 0：{v2}"
            );
        }
    }
}

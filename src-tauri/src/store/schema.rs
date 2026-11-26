//! SQLite schema and forward-only migrations.
//!
//! Version 1 lays down every table later milestones need (routing, usage,
//! logs, cache, skills) so no milestone has to race a migration of someone
//! else's table.

use rusqlite::Connection;

/// Applied schema version. Bump by appending to [`MIGRATIONS`].
///
/// 直接取 `MIGRATIONS.len()`：写死数字迟早跟数组长度分叉（本文件曾长期写着 6
/// 而数组已有 8 段），而这个常量唯一的用处就是验数组没被漏加——让它自证比手抄强。
const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

const STATEMENTS_V1: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS backends (
        id                   INTEGER PRIMARY KEY AUTOINCREMENT,
        name                 TEXT NOT NULL UNIQUE,
        base_url             TEXT NOT NULL,
        protocol             TEXT NOT NULL,
        api_key              TEXT,
        weight               INTEGER NOT NULL DEFAULT 1,
        enabled              INTEGER NOT NULL DEFAULT 1,
        engine               TEXT,
        webui_url            TEXT,
        health_path          TEXT,
        models               TEXT NOT NULL DEFAULT '[]',
        aliases              TEXT NOT NULL DEFAULT '{}',
        capabilities         TEXT NOT NULL DEFAULT '{}',
        params               TEXT NOT NULL DEFAULT '{}',
        health_state         TEXT NOT NULL DEFAULT 'unknown',
        consecutive_failures INTEGER NOT NULL DEFAULT 0,
        last_checked_at      INTEGER,
        last_error           TEXT,
        created_at           INTEGER NOT NULL,
        updated_at           INTEGER NOT NULL
    )
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS backend_models (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        backend_id   INTEGER NOT NULL REFERENCES backends(id) ON DELETE CASCADE,
        public_name  TEXT NOT NULL,
        upstream_name TEXT NOT NULL,
        enabled      INTEGER NOT NULL DEFAULT 1,
        UNIQUE (backend_id, public_name)
    )
    "#,
    r#"
    -- Layered parameter overrides: global -> backend -> model (plan §5.3).
    CREATE TABLE IF NOT EXISTS param_overrides (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        scope      TEXT NOT NULL,
        ref_id     TEXT NOT NULL DEFAULT '',
        key        TEXT NOT NULL,
        value      TEXT NOT NULL,
        policy     TEXT NOT NULL DEFAULT 'fill',
        updated_at INTEGER NOT NULL,
        UNIQUE (scope, ref_id, key)
    )
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS usage_records (
        id                   INTEGER PRIMARY KEY AUTOINCREMENT,
        request_id           TEXT NOT NULL,
        ts                   INTEGER NOT NULL,
        backend              TEXT,
        model                TEXT,
        entry_protocol       TEXT,
        client               TEXT,
        status               INTEGER NOT NULL DEFAULT 0,
        ok                   INTEGER NOT NULL DEFAULT 1,
        stream               INTEGER NOT NULL DEFAULT 0,
        ttft_ms              INTEGER,
        duration_ms          INTEGER,
        input_tokens         INTEGER NOT NULL DEFAULT 0,
        output_tokens        INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens    INTEGER NOT NULL DEFAULT 0,
        cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
        decode_tps           REAL,
        estimated            INTEGER NOT NULL DEFAULT 0,
        cache_status         TEXT,
        retry_trace          TEXT,
        error                TEXT
    )
    "#,
    "CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage_records (ts)",
    "CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_records (model, ts)",
    r#"
    CREATE TABLE IF NOT EXISTS usage_daily (
        day          TEXT NOT NULL,
        backend      TEXT NOT NULL DEFAULT '',
        model        TEXT NOT NULL DEFAULT '',
        requests     INTEGER NOT NULL DEFAULT 0,
        input_tokens INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens INTEGER NOT NULL DEFAULT 0,
        ttft_sum     INTEGER NOT NULL DEFAULT 0,
        ttft_count   INTEGER NOT NULL DEFAULT 0,
        tps_sum      REAL NOT NULL DEFAULT 0,
        tps_count    INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, backend, model)
    )
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS logs (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        ts         INTEGER NOT NULL,
        level      TEXT NOT NULL,
        code       TEXT,
        message    TEXT NOT NULL,
        request_id TEXT,
        backend    TEXT,
        model      TEXT,
        extra      TEXT NOT NULL DEFAULT '{}'
    )
    "#,
    "CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs (ts)",
    "CREATE INDEX IF NOT EXISTS idx_logs_level_ts ON logs (level, ts)",
    r#"
    -- Cache policy layers: global -> backend -> model (plan §5.5.3).
    CREATE TABLE IF NOT EXISTS cache_policies (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        scope      TEXT NOT NULL,
        ref_id     TEXT NOT NULL DEFAULT '',
        settings   TEXT NOT NULL DEFAULT '{}',
        updated_at INTEGER NOT NULL,
        UNIQUE (scope, ref_id)
    )
    "#,
    r#"
    -- Index of the disk tier; the payload itself lives in the cache dir.
    CREATE TABLE IF NOT EXISTS cache_entries (
        key        TEXT PRIMARY KEY,
        model      TEXT,
        bytes      INTEGER NOT NULL DEFAULT 0,
        hits       INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        tier       TEXT NOT NULL DEFAULT 'disk'
    )
    "#,
    r#"
    -- prefix_hash -> which backend is believed to hold it warm (§5.5.7).
    CREATE TABLE IF NOT EXISTS cache_affinity (
        prefix_hash TEXT NOT NULL,
        backend_id  INTEGER NOT NULL,
        confidence  REAL NOT NULL DEFAULT 1.0,
        last_seen   INTEGER NOT NULL,
        PRIMARY KEY (prefix_hash, backend_id)
    )
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS skills (
        dir         TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        description TEXT,
        repo        TEXT,
        branch      TEXT,
        version     TEXT,
        latest      TEXT,
        size        INTEGER NOT NULL DEFAULT 0,
        managed     INTEGER NOT NULL DEFAULT 1,
        updated_at  INTEGER NOT NULL
    )
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS skill_apps (
        skill_dir TEXT NOT NULL,
        app_id    TEXT NOT NULL,
        enabled   INTEGER NOT NULL DEFAULT 1,
        sync_mode TEXT NOT NULL DEFAULT 'symlink',
        PRIMARY KEY (skill_dir, app_id)
    )
    "#,
    "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    r#"
    CREATE TABLE IF NOT EXISTS events (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        ts      INTEGER NOT NULL,
        kind    TEXT NOT NULL,
        payload TEXT NOT NULL DEFAULT '{}'
    )
    "#,
];

/// Ordered migrations: index 0 is the migration to version 1, and so on.
const STATEMENTS_V2: &[&str] = &[
    // M4-B: 日聚合补上错误计数（错误率趋势用）。
    "ALTER TABLE usage_daily ADD COLUMN errors INTEGER NOT NULL DEFAULT 0",
];

const STATEMENTS_V3: &[&str] = &[
    // M6-B: Skills 仓库清单（发现与安装的来源登记表）。
    r#"
    CREATE TABLE IF NOT EXISTS skill_repos (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        owner   TEXT NOT NULL,
        name    TEXT NOT NULL,
        branch  TEXT NOT NULL DEFAULT 'main',
        enabled INTEGER NOT NULL DEFAULT 1,
        UNIQUE(owner, name)
    )
    "#,
];

const STATEMENTS_V4: &[&str] = &[
    // M7: 后端控制台入口与免登录凭据（均可空；随 base_url/api_key 一同保存）。
    "ALTER TABLE backends ADD COLUMN webui_user TEXT",
    "ALTER TABLE backends ADD COLUMN webui_password TEXT",
];

const STATEMENTS_V5: &[&str] = &[
    // M12-A：每模型暴露开关升成一等公民。
    // 旧实现把「后端 enabled」顺手写进了 backend_models.enabled，两件事混在一列上；
    // 先归一成「全部暴露」，此后这一列只由模型自己的开关拥有。
    "UPDATE backend_models SET enabled = 1",
    // 改名后的过渡开关：是否同时接受上游原名。
    "ALTER TABLE backend_models ADD COLUMN accept_upstream INTEGER NOT NULL DEFAULT 0",
];

const STATEMENTS_V6: &[&str] = &[
    // 模型元数据声明（ctx/output）：0 = 自动（按后端设定值走），>0 = 人工强制覆盖。
    // 按对外暴露名全局唯一，跨后端共享（多后端同一模型只声明一次）。
    // 只有「人工写了非 0」才有行：0/缺省等价，删行即回到自动（决策「缺省设置为 0 值」）。
    r#"
    CREATE TABLE IF NOT EXISTS model_meta (
        public_name TEXT PRIMARY KEY,
        ctx         INTEGER NOT NULL DEFAULT 0,
        output      INTEGER NOT NULL DEFAULT 0,
        updated_at  INTEGER NOT NULL
    )
    "#,
    // 扫描探测到的上游真值，按「后端 × 模型」存：NULL = 上游没给，不猜。
    // 放 backend_models 而不是 model_meta：探测是后端侧事实，跟随行的改名/摘除自然迁移。
    "ALTER TABLE backend_models ADD COLUMN probe_ctx INTEGER",
    "ALTER TABLE backend_models ADD COLUMN probe_output INTEGER",
];

const STATEMENTS_V7: &[&str] = &[
    // 探测到的 reasoning/vlm 能力位（三态）：NULL = 上游没说，不猜；1/0 = 明说开/关。
    // 与 probe_ctx 同理放 backend_models：能力是后端侧事实（量化裁剪会丢视觉）。
    "ALTER TABLE backend_models ADD COLUMN probe_reasoning INTEGER",
    "ALTER TABLE backend_models ADD COLUMN probe_vlm INTEGER",
    // 人工三态声明：0 = 自动（跟随探测），1 = 强制开，2 = 强制关。
    // 与 ctx/output 的「0 = 自动」同族语义；行存在条件放宽为「任一字段非 0」。
    "ALTER TABLE model_meta ADD COLUMN reasoning INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE model_meta ADD COLUMN vlm INTEGER NOT NULL DEFAULT 0",
];

const STATEMENTS_V8: &[&str] = &[
    // 片4 · 手工单价（USD / 百万 token）：纯手工进 model_meta，NULL = 未设 = 不算；
    // 只对在线后端（engine: custom/未填）有意义，本地引擎按引擎自动判定不计费（D25）。
    "ALTER TABLE model_meta ADD COLUMN price_input REAL",
    "ALTER TABLE model_meta ADD COLUMN price_output REAL",
    "ALTER TABLE model_meta ADD COLUMN price_cache_read REAL",
    "ALTER TABLE model_meta ADD COLUMN price_cache_creation REAL",
];

const STATEMENTS_V9: &[&str] = &[
    // 控制台页「入口设置」卡的启停脚本（用户填的本机脚本路径，可空）。
    // 只存路径不存命令行——执行时直接 execve 该文件（见 services::script），
    // 不拼 shell 字符串，杜绝把配置变成注入面。
    "ALTER TABLE backends ADD COLUMN start_script TEXT",
    "ALTER TABLE backends ADD COLUMN stop_script TEXT",
];

const STATEMENTS_V10: &[&str] = &[
    // 互斥引擎配对（lan-qwen 这类「同一服务的两种引擎实现」）：alt_engine 记配对引擎名。
    // 当前生效引擎就是 engine 列本身——切换 = engine 与 alt_engine 互换（见 commands::halctl），
    // 能力表/协议/面板分流全部随 engine 自动派生，不再造第二份「当前态」真相。
    // 与 start_script 同口径：只存 DB 不进 yaml（BackendCfg 无此字段，播种/镜像都不碰它）。
    "ALTER TABLE backends ADD COLUMN alt_engine TEXT",
];

const STATEMENTS_V11: &[&str] = &[
    // 模型工种类（媒体端点透传代理 11a）：chat / image_gen / tts / asr / video_gen，
    // 与 chat 并列互斥归类。NULL/空 = 未声明（回落判定链：人工 > /health > 名字建议 > chat，
    // 见 gateway::model_kind）。工种只作用于被选入的模型——媒体工种永不进聊天择路与分配格子。
    "ALTER TABLE backend_models ADD COLUMN kind TEXT",
];

const STATEMENTS_V12: &[&str] = &[
    // 媒体计量（媒体端点透传代理 11c）：图像张数与音频秒数，挂在现有用量表上
    // （计划口径「用量记录复用现有落库」）。NULL = 取不到，UI 显示 —，不造假数字；
    // 有值的行即媒体行（token 列恒 0，entry_protocol 存媒体端点路径）。
    "ALTER TABLE usage_records ADD COLUMN media_images INTEGER",
    "ALTER TABLE usage_records ADD COLUMN media_seconds REAL",
];

const STATEMENTS_V13: &[&str] = &[
    // 提示词速取浮动面板（计划 2026-11-18）：常用提示词与各代理快捷命令。
    // tab = "common"（通用库）或某代理的 ClientApp::as_str()；sort 为 tab 内
    // 手工顺序（新条目追加末尾）。内容原样存文本，注入时逐字进剪贴板。
    r#"CREATE TABLE IF NOT EXISTS prompt_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        tab TEXT NOT NULL,
        content TEXT NOT NULL,
        sort INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_prompt_items_tab ON prompt_items(tab, sort);"#,
];

const STATEMENTS_V14: &[&str] = &[
    // 提示词使用计数（用户：「自动把高频用的排前面」）。use_count = 取用次数，
    // last_used_at = 最后一次取用；两者只用于排序，不参与任何业务判定。
    // 默认 0 意味着预置条目与老数据同一起跑线，手工 sort 仍是同分时的决胜项。
    "ALTER TABLE prompt_items ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE prompt_items ADD COLUMN last_used_at INTEGER NOT NULL DEFAULT 0",
];

const STATEMENTS_V15: &[&str] = &[
    // 后端直连模式（计划 2026-11-18-backend-direct-mode）：三态「停用/网关/直连」里
    // 停用复用 enabled=0（不双轨），本列只区分 网关(0)/直连(1)。
    "ALTER TABLE backends ADD COLUMN direct INTEGER NOT NULL DEFAULT 0",
];

const MIGRATIONS: &[&[&str]] = &[
    STATEMENTS_V1,
    STATEMENTS_V2,
    STATEMENTS_V3,
    STATEMENTS_V4,
    STATEMENTS_V5,
    STATEMENTS_V6,
    STATEMENTS_V7,
    STATEMENTS_V8,
    STATEMENTS_V9,
    STATEMENTS_V10,
    STATEMENTS_V11,
    STATEMENTS_V12,
    STATEMENTS_V13,
    STATEMENTS_V14,
    STATEMENTS_V15,
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)",
    )?;
    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;

    for (idx, statements) in MIGRATIONS.iter().enumerate() {
        let target = idx as i64 + 1;
        if current >= target {
            continue;
        }
        conn.execute_batch("BEGIN")?;
        let applied = (|| -> rusqlite::Result<()> {
            for sql in *statements {
                conn.execute_batch(sql)?;
            }
            conn.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![target, crate::types::now_ms()],
            )?;
            Ok(())
        })();
        match applied {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(err);
            }
        }
    }

    debug_assert!(SCHEMA_VERSION as usize <= MIGRATIONS.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent_and_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count >= 13, "expected all tables, got {count}");
        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);

        // V9 的启停脚本两列必须真落到位（迁移跑完就该在，前端要读）。
        for col in ["start_script", "stop_script"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('backends') WHERE name = ?1",
                    [col],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "backends 缺列 {col}");
        }

        // V15 的直连列必须真落到位（三态取用要读它）。
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('backends') WHERE name = 'direct'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "backends 缺列 direct");

        // V13 的提示词速取表必须真建出来（浮窗启动即读）。
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='prompt_items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "prompt_items 表缺失");
    }
}

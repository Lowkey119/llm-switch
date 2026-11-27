// 日志卡内容（从 HalogenPanel 原样搬出）：等级/行数/跟随控制条 + 日志 pre +
// 自动滚底。日志卡双用途：默认日志，点「参数设置」临时切成调参窗口（用户拍板
// 的复用方式）——调参时 logs query 停轮询省 SSH（enabled 由面板的 active 传入）。
import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, fieldCls } from "../../components/ui";
import { every, POLL } from "../../lib/poll";
import { ipc, type HalLogs } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import { AutoScroll, LogLine } from "./parts";

export function LogsCard({ ch, active }: { ch: string; active: boolean }) {
  const [level, setLevel] = useState("info");
  const [tail, setTail] = useState(500);
  const [follow, setFollow] = useState(true);

  const logs = useQuery({
    queryKey: ["hal", ch, "logs", level, tail],
    queryFn: () => ipc<HalLogs>("halctl_logs", { ch, tail, level }),
    refetchInterval: () => (follow ? every(POLL.tail) : false),
    // 调参视图占用同一张卡时停轮询，省 SSH。
    enabled: active,
  });

  const logRef = useRef<HTMLPreElement>(null);
  const logLines = logs.data?.lines ?? [];

  return (
    <>
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="font-medium text-ink-2">{t("page.logs")}</span>
        <select className={fieldCls} value={level} onChange={(e) => setLevel(e.target.value)}>
          <option value="info">{t("hl.lvlInfo")}</option>
          <option value="debug">{t("hl.lvlDebug")}</option>
          <option value="trace">{t("hl.lvlTrace")}</option>
          <option value="raw">{t("hl.lvlRaw")}</option>
        </select>
        <select className={fieldCls} value={tail} onChange={(e) => setTail(Number(e.target.value))}>
          {[200, 500, 1000, 2000].map((n) => (
            <option key={n} value={n}>
              {ts("hl.rowsN", { n })}
            </option>
          ))}
        </select>
        <label className="flex cursor-pointer items-center gap-1 text-ink-3">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} />
          {t("hl.autoRefresh")}
        </label>
        <Button onClick={() => logs.refetch()}>{t("cs.refresh")}</Button>
      </div>
      <pre
        ref={logRef}
        onScroll={(e) => {
          // 用户往上翻就暂停自动滚底，回到底部再恢复跟随。
          const el = e.currentTarget;
          const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          el.dataset.atBottom = atBottom ? "1" : "0";
        }}
        className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all rounded-md border border-slate-800 bg-slate-950/70 p-2 font-mono text-xs leading-relaxed"
      >
        {logLines.length ? (
          logLines.map((line, i) => <LogLine key={i} line={line} />)
        ) : logs.data?.error ? (
          // 空日志有两种：真没行，和读不到（引擎停止时容器被删）。后者必须说清，
          // 不然用户以为日志功能坏了。
          <span className="text-[var(--sem-danger-text)]">{logs.data.error}</span>
        ) : (
          <span className="text-ink-4">{t("hl.noLines")}</span>
        )}
      </pre>
      <AutoScroll pre={logRef} lines={logLines.length} enabled={follow} />
    </>
  );
}

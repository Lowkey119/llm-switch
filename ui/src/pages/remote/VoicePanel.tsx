// 语音录入卡（计划 slot:S-F4）：语音键按住 = 一次采集会话，松手收尾，出口四选一。
//
// 路 1 系统转写（缺省）：音频源是 Mac 本机麦克风，partial 实时回显、定稿注入焦点框；
// 路 2 虚拟声卡透传（配用户自有输入法）；路 3 网关 ASR 转写 + 文本注入。
// 快照一次取齐走 remote_voice_get；会话进行中的状态与错误走 remote://voice 事件。
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Card, CardHead, FieldLabel, SkeletonRows, StatusDot, fieldCls } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { CopyButton } from "../../components/CopyButton";
import { IconMic, IconSettings } from "../../components/icons";
import { t, ts } from "../../lib/i18n";
import { ipc, ipcEvent, type BackendRow } from "../../lib/ipc";
import { resolveWorkKind } from "../../lib/workkind";
import { LevelMeter, Metric } from "./VoiceMeter";

/** 与后端 VoiceRoute 对齐（snake_case）。 */
type Route = "system" | "soundcard" | "gateway" | "off";
/** 与后端 VoiceState 对齐。 */
type VoiceState = "idle" | "listening" | "transcribing" | "injected";

interface Settings {
  route: Route;
  device: string | null;
  /** 输出音量 0…1000（1000% ≈ +20 dB）。 */
  gain: number;
  stt_model: string | null;
  /** 路 2：接听语音的输入法 id（会话开始切过去、按住 🌐 触发、收尾还原）。 */
  ime_source: string | null;
}

/** 上一次会话的实测指标；取不到的字段后端给 null，页面显示 —。 */
interface Metrics {
  frames: number;
  samples: number;
  ms: number;
  rate: number;
  coverage: number | null;
  first_pcm_ms: number | null;
}

interface Snapshot {
  state: VoiceState;
  settings: Settings;
  metrics: Metrics | null;
  note: string | null;
  devices: { devices: string[]; default: string };
}

/** remote://voice 事件：状态/错误/定稿文本、系统转写的实时 partial，以及采集中的实时电平（节流 30ms）。 */
interface VoiceEvent {
  state?: VoiceState;
  error?: string;
  text?: string;
  partial?: string;
  level?: number;
}

/** 与后端 ime::ImeSource 对齐。 */
interface ImeSource {
  id: string;
  name: string;
}

/** 电平波形的滚动窗（30ms 一发 ≈ 7 s 回看）。 */
const LEVELS = 240;

/** 状态灯：形状走 StatusDot，文字必须同屏（法条：状态不得只靠颜色表达）。 */
const STATE: Record<VoiceState, { dot: string; busy?: boolean; key: string }> = {
  idle: { dot: "unknown", key: "remote.voice.idle" },
  listening: { dot: "healthy", busy: true, key: "remote.voice.listening" },
  transcribing: { dot: "healthy", busy: true, key: "remote.voice.transcribing" },
  injected: { dot: "healthy", key: "remote.voice.injected" },
};

const withUnit = (v: number | null | undefined, unit: string) => (v == null ? "—" : `${v}${unit}`);

export function VoicePanel() {
  const qc = useQueryClient();
  const toast = useToast();
  const q = useQuery({
    queryKey: ["remote-voice"],
    queryFn: () => ipc<Snapshot>("remote_voice_get"),
  });
  // 事件比回包快：会话中的状态/错误先由事件驱动，快照随后兜底。
  // partial 是系统转写的实时文本，只在路 1 出现；其余事件一到就清空（不留旧半句）。
  const [live, setLive] = useState<{
    state: VoiceState;
    note: string | null;
    text: string | null;
    partial: string | null;
  }>({ state: "idle", note: null, text: null, partial: null });
  // 采集中的实时电平（0…1）。松手/失败的状态事件一到就清空：留着一条不动的曲线
  // 看着像还在收，那是假状态。
  const [levels, setLevels] = useState<number[]>([]);
  // 乐观覆盖：保存失败即回退快照值，绝不留「页面说了、后端没存」的假状态。
  const [draft, setDraft] = useState<Settings | null>(null);
  // 本地卡（虚拟声卡/系统转写并一张，用户 2026-11-21 拍板）组合框记住上次选择：
  // 关路再开回到上次那路；缺省虚拟声卡（系统转写垫底）。
  const [lastLocal, setLastLocal] = useState<"soundcard" | "system">("soundcard");

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let dead = false;
    void ipcEvent<VoiceEvent>("remote://voice", (e) => {
      if (typeof e.level === "number") {
        const lv = e.level;
        setLevels((prev) => [...prev.slice(-(LEVELS - 1)), lv]);
        return;
      }
      if (typeof e.partial === "string") {
        // 实时回显只改 partial：不清电平、不触发快照重取（partial 一发一条，别淹 IPC）。
        const p = e.partial;
        setLive((prev) => ({ ...prev, partial: p }));
        return;
      }
      setLevels([]);
      setLive({ state: e.state ?? "idle", note: e.error ?? null, text: e.text ?? null, partial: null });
      void qc.invalidateQueries({ queryKey: ["remote-voice"] });
    }).then((u) => {
      if (dead) u();
      else unlisten = u;
    });
    return () => {
      dead = true;
      unlisten?.();
    };
  }, [qc]);

  const snap = q.data;
  const settings = draft ?? snap?.settings ?? null;
  // 本地卡当前子路：正走本地两路时随 route，否则回上次选择（缺省虚拟声卡）。
  const localSel: "soundcard" | "system" =
    settings?.route === "system" || settings?.route === "soundcard" ? settings.route : lastLocal;
  const isLocal = settings?.route === "system" || settings?.route === "soundcard";

  const save = useMutation({
    mutationFn: (next: Settings) => ipc("remote_voice_set", { settings: next }),
    onSuccess: () => {
      setDraft(null);
      void qc.invalidateQueries({ queryKey: ["remote-voice"] });
    },
    onError: (e) => {
      setDraft(null);
      toast.err(String(e));
    },
  });

  const patch = (p: Partial<Settings>) => {
    if (!settings) return;
    const next = { ...settings, ...p };
    setDraft(next);
    save.mutate(next);
  };

  // 路 3 的模型清单 = 网关暴露模型里工种判定为 asr 的那些（与后端择路同一口径 lib/workkind）。
  const gateways = useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
    enabled: settings?.route === "gateway",
  });
  const asrModels = useMemo(() => {
    const out = new Set<string>();
    for (const b of gateways.data ?? []) {
      if (!b.enabled) continue;
      for (const m of b.models) {
        const flag = b.model_flags?.find((f) => f.public_name === m);
        if (resolveWorkKind(flag, m) === "asr") out.add(m);
      }
    }
    return [...out].sort();
  }, [gateways.data]);

  // 路 2 输入法清单（Carbon TIS 已启用可选项）：输入法组合框数据源。
  // 缺配置就在卡内就地提醒（用户 2026-11-21：不再另立全局提醒条，重复）；
  // 系统授权问题走「授权检查」统一入口。
  const imes = useQuery({
    queryKey: ["remote-ime"],
    queryFn: () => ipc<ImeSource[]>("remote_ime_list"),
  });
  const imeName = imes.data?.find((i) => i.id === settings?.ime_source)?.name ?? null;

  // 事件驱动为主、快照兜底：页面中途打开（或热重载）时开场事件早就错过了，
  // 只认 live 会把「正在聆听」显示成空闲——注释承诺的兜底得真的兜上。
  const state: VoiceState = live.state !== "idle" ? live.state : (snap?.state ?? "idle");
  const st = STATE[state];
  const devices = snap?.devices ?? { devices: [], default: "" };
  const note = live.note ?? snap?.note ?? null;
  const m = snap?.metrics ?? null;

  // 转写方式组合框：声卡分支进两列栅格、系统转写分支独立一行，两处共用一份。
  const localPick = (
    <FieldLabel row label={t("remote.voice.localPick")}>
      <select
        className={`${fieldCls} min-w-0 flex-1`}
        value={localSel}
        onChange={(e) => {
          const v = e.target.value as "soundcard" | "system";
          setLastLocal(v);
          patch({ route: v });
        }}
      >
        <option value="soundcard">{t("remote.voice.soundcard")}</option>
        <option value="system">{t("remote.voice.system")}</option>
      </select>
    </FieldLabel>
  );

  return (
    <div className="space-y-3">
      {/* 顶部并两张卡（用户 2026-11-21 改版）：左 = 语音键状态 + 各路共用波形，
          右 = 实时回显；路 2 转写走输入法 HUD 上屏、应用拿不到文字，回显卡直接隐藏，
          波形卡自动占满整行（cards-2 单卡即通栏）。 */}
      <div className="cards-2 gap-3">
        <Card>
          <CardHead icon={IconMic} title={t("remote.voice.title")}>
            <span className="flex items-center gap-1.5 text-sm text-ink-2">
              <StatusDot state={st.dot} busy={st.busy} text={t(st.key)} />
              {t(st.key)}
            </span>
            {/* 关路显式入口（用户 2026-11-21 拍板）：卡片是单选，点已选中的卡不再取消，
                关闭走这里的按钮。 */}
            {settings && settings.route !== "off" && (
              <Button size="sm" title={t("remote.voice.offHint")} onClick={() => patch({ route: "off" })}>
                {t("remote.voice.off")}
              </Button>
            )}
          </CardHead>
          {q.isPending && <SkeletonRows rows={2} />}
          {q.isError && (
            <QueryError inline title={t("remote.voice.loadFail")} error={q.error} onRetry={() => void q.refetch()} />
          )}
          {settings && settings.route !== "off" && (
            <div className="flex items-center gap-3">
              <div className="min-w-0 flex-1">
                <LevelMeter levels={levels} />
              </div>
              <span
                className="w-12 shrink-0 text-right text-sm tabular-nums text-ink-4"
                title={t("remote.voice.levelHint")}
              >
                {levels.length ? `${Math.round(levels[levels.length - 1] * 100)}%` : "—"}
              </span>
            </div>
          )}
          {settings && note && (
            <p className="text-sm leading-relaxed text-[var(--sem-danger-text)]" role="alert">
              {note}
            </p>
          )}
          {/* 上次会话实测指标（用户 2026-11-21 改版）：挪进波形卡底部；没跑过会话一律 —。 */}
          {settings && settings.route !== "off" && (
            <dl className="flex flex-wrap items-baseline gap-x-6 gap-y-1 border-t border-slate-800 pt-3 text-sm">
              <span className="text-ink-4">{t("remote.voice.lastSession")}</span>
              <Metric label={t("remote.voice.mMs")} value={withUnit(m?.ms, " ms")} />
              <Metric label={t("remote.voice.mRate")} value={withUnit(m?.rate, " Hz")} />
              <Metric label={t("remote.voice.mFrames")} value={m ? String(m.frames) : "—"} />
              <Metric label={t("remote.voice.mFirst")} value={withUnit(m?.first_pcm_ms, " ms")} />
              <Metric
                label={t("remote.voice.mCoverage")}
                value={m?.coverage == null ? "—" : `${m.coverage.toFixed(1)}%`}
              />
            </dl>
          )}
        </Card>
        {settings && settings.route !== "soundcard" && (
          <Card>
            <div className="mb-1 flex items-baseline justify-between gap-3">
              <span className="text-sm text-ink-3" title={t("remote.voice.echoHint")}>
                {t("remote.voice.echo")}
              </span>
              {live.text && (
                <CopyButton text={live.text} label={t("remote.voice.copyText")} className="shrink-0" />
              )}
            </div>
            <div
              aria-live="polite"
              className="min-h-20 break-words whitespace-pre-wrap rounded-md border border-slate-800 px-2.5 py-2 text-sm leading-relaxed text-ink-1"
            >
              {live.partial ?? live.text ?? "—"}
            </div>
          </Card>
        )}
      </div>

      {settings && (
        <>
          {/* 路径卡（用户 2026-11-21 改版）：本地两路并一张卡——虚拟声卡缺省在前、
              系统转写垫底，组合框切换；网关转写未调通，只作手动备选单独一张。
              点卡头选路（单选，点已选中的卡不取消，关路走波形卡头「关闭」按钮）；卡体控件 stopPropagation——改配置不顺手切路。 */}
          <div className="cards-2 gap-3" role="radiogroup" aria-label={t("remote.voice.route")}>
            <RouteCard
              title={t("remote.voice.local")}
              hint={t("remote.voice.localHint")}
              active={isLocal}
              onSelect={() => patch({ route: localSel })}
              /* 输入法就绪回显挪进卡头（用户 2026-11-21）：徽标左侧，卡底不再占行。 */
              status={
                localSel === "soundcard" ? (
                  imeName ? (
                    <span className="flex min-w-0 items-center gap-1.5 text-sm text-[var(--sem-ok-text)]">
                      <StatusDot state="healthy" text={ts("remote.voice.imeReady", { name: imeName })} />
                      {ts("remote.voice.imeReady", { name: imeName })}
                    </span>
                  ) : (
                    <span className="text-sm text-[var(--sem-busy-text)]">{t("remote.voice.imeMissing")}</span>
                  )
                ) : null
              }
            >
              {localSel === "soundcard" ? (
                /* 卡内固定两列（用户 2026-11-21 圈定）：转写方式|输出设备、输入法|输出音量。
                   不用 form-grid——其 300px 最小列宽在这卡宽下装不下两列，会回落单列。
                   控件统一 h-7：原生 select 与 number 输入默认行高不同，不齐就穿帮。 */
                <div className="grid grid-cols-2 gap-x-3 gap-y-2">
                  {localPick}
                  <FieldLabel row label={t("remote.voice.device")} hint={t("remote.voice.deviceHint")}>
                    <select
                      className={`${fieldCls} min-w-0 flex-1`}
                      value={settings.device ?? ""}
                      onChange={(e) => patch({ device: e.target.value || null })}
                    >
                      <option value="">{t("remote.voice.deviceNone")}</option>
                      {devices.devices.map((d) => (
                        <option key={d} value={d}>
                          {d}
                          {d === devices.default ? ` ${t("remote.voice.deviceDefault")}` : ""}
                        </option>
                      ))}
                    </select>
                  </FieldLabel>
                  <FieldLabel row label={t("remote.voice.ime")} hint={t("remote.voice.imeHint")}>
                    <span className="flex min-w-0 flex-1 items-center gap-1.5">
                      <select
                        className={`${fieldCls} min-w-0 flex-1`}
                        value={settings.ime_source ?? ""}
                        onChange={(e) => patch({ ime_source: e.target.value || null })}
                      >
                        <option value="">{t("remote.voice.imeNone")}</option>
                        {(imes.data ?? []).map((i) => (
                          <option key={i.id} value={i.id}>
                            {i.name}
                          </option>
                        ))}
                      </select>
                      <Button
                        size="sm"
                        className="shrink-0"
                        title={t("remote.voice.imeOpen")}
                        aria-label={t("remote.voice.imeOpen")}
                        onClick={() => void ipc("remote_ime_open_settings").catch((e) => toast.err(String(e)))}
                      >
                        <IconSettings className="h-3.5 w-3.5" />
                      </Button>
                    </span>
                  </FieldLabel>
                  <FieldLabel row label={t("remote.voice.gain")} hint={t("remote.voice.gainHint")}>
                    <input
                      className={`${fieldCls} min-w-0 flex-1`}
                      type="number"
                      min={0}
                      max={1000}
                      step={10}
                      value={settings.gain}
                      onChange={(e) => patch({ gain: Math.min(1000, Math.max(0, Number(e.target.value) || 0)) })}
                    />
                  </FieldLabel>
                </div>
              ) : (
                <>
                  {localPick}
                  <p className="text-sm leading-relaxed text-ink-4">{t("remote.voice.systemHint")}</p>
                </>
              )}
            </RouteCard>
            <RouteCard
              title={t("remote.voice.gateway")}
              hint={t("remote.voice.gatewayHint")}
              active={settings.route === "gateway"}
              onSelect={() => patch({ route: "gateway" })}
            >
              <div className="form-grid gap-x-3 gap-y-2">
                <FieldLabel row label={t("remote.voice.model")} hint={t("remote.voice.modelHint")}>
                  {gateways.isPending ? (
                    <SkeletonRows rows={1} />
                  ) : asrModels.length === 0 ? (
                    <span className="text-sm text-ink-4">{t("remote.voice.modelEmpty")}</span>
                  ) : (
                    <select
                      className={`${fieldCls} min-w-0 flex-1`}
                      value={settings.stt_model ?? ""}
                      onChange={(e) => patch({ stt_model: e.target.value || null })}
                    >
                      <option value="">{t("remote.voice.modelNone")}</option>
                      {asrModels.map((m) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))}
                    </select>
                  )}
                </FieldLabel>
                <FieldLabel row label={t("remote.voice.gain")} hint={t("remote.voice.gainHint")}>
                  <input
                    className={`${fieldCls} min-w-0 flex-1`}
                    type="number"
                    min={0}
                    max={1000}
                    step={10}
                    value={settings.gain}
                    onChange={(e) => patch({ gain: Math.min(1000, Math.max(0, Number(e.target.value) || 0)) })}
                  />
                </FieldLabel>
              </div>
            </RouteCard>
          </div>
          {settings.route === "off" && (
            <p className="text-sm text-ink-4">{t("remote.voice.offHint")}</p>
          )}
        </>
      )}
    </div>
  );
}

/** 路径卡：点卡头 = 选路（radiogroup 单选）；激活卡荧光青描边 + 「使用中」徽标；
 *  卡体控件 stopPropagation——在未激活卡上改配置不会顺手切路。 */
function RouteCard({
  title,
  hint,
  active,
  onSelect,
  status,
  children,
}: {
  title: string;
  hint: string;
  active: boolean;
  onSelect: () => void;
  /** 卡头状态位（用户 2026-11-21）：渲染在「使用中」徽标左侧，如输入法就绪回显。 */
  status?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section
      role="radio"
      aria-checked={active}
      tabIndex={0}
      title={hint}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={`cursor-pointer rounded-xl border p-4 ${
        active ? "border-[var(--key-hl)] shadow-[0_0_0_1px_var(--key-hl)]" : "border-slate-800 hover:border-slate-600"
      }`}
    >
      <div className="mb-3 flex items-center gap-2">
        <span
          aria-hidden
          className={`h-3.5 w-3.5 shrink-0 rounded-full border-2 ${
            active ? "border-[var(--key-hl)] bg-[radial-gradient(var(--key-hl)_0_3px,transparent_4px)]" : "border-slate-600"
          }`}
        />
        <h3 className="text-sm font-semibold text-ink-1">{title}</h3>
        <span className="ml-auto flex min-w-0 items-center gap-2">
          {status}
          <span
            className={`shrink-0 rounded border px-1.5 text-2xs ${
              active ? "border-[var(--sem-ok-text)] text-[var(--sem-ok-text)]" : "border-slate-700 text-ink-4"
            }`}
          >
            {active ? t("remote.voice.inUse") : t("remote.voice.notInUse")}
          </span>
        </span>
      </div>
      <div className="space-y-2" onClick={(e) => e.stopPropagation()}>
        {children}
      </div>
    </section>
  );
}

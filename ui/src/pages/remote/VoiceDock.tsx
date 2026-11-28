// 语音坞（用户 2026-11-24 定稿）：原「语音录入设置」对话框内容内联进映射页右框，
// 不再弹层——路由 / 输出设备 / 输入法 / 增益就地配置，平时显示纵向音频波形柱状条。
//
// 数据层与原 VoicePanel 同源：remote_voice_get 快照一次取齐，会话中的状态 / 错误 /
// 实时电平走 remote://voice 事件（电平节流 30ms）。乐观保存：失败即回退快照值。
import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, FieldLabel, SkeletonRows, StatusDot, Toggle, fieldCls } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { IconLock, IconMic, IconSettings } from "../../components/icons";
import { t, ts } from "../../lib/i18n";
import { ipc, ipcEvent, type BackendRow } from "../../lib/ipc";
import { resolveWorkKind } from "../../lib/workkind";
import { paneCls } from "./MacroLibrary";
import { VBars as BitsVBars } from "./remoteBits";

/** 与后端 VoiceRoute 对齐（snake_case）。 */
type Route = "system" | "soundcard" | "gateway" | "off";
/** 与后端 VoiceState 对齐。 */
type VoiceState = "idle" | "listening" | "transcribing" | "injected";

interface Settings {
  route: Route;
  device: string | null;
  /** 输出音量 0…1000（1000% ≈ +20 dB）。 */
  gain: number;
  /** 每台遥控器独立增益（档案 id → 百分比），后端持久化。 */
  gain_by_profile?: Record<string, number>;
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
/** 语音设置编辑锁的持久化键（默认锁住，用户 2026-11-25：开锁才能改下面的设置）。 */
const LOCK_KEY = "llm-switch.voice.lock";

/** 状态灯：形状走 StatusDot，文字必须同屏（法条：状态不得只靠颜色表达）。 */
const STATE: Record<VoiceState, { dot: string; busy?: boolean; key: string }> = {
  idle: { dot: "unknown", key: "remote.voice.idle" },
  listening: { dot: "healthy", busy: true, key: "remote.voice.listening" },
  transcribing: { dot: "healthy", busy: true, key: "remote.voice.transcribing" },
  injected: { dot: "healthy", key: "remote.voice.injected" },
};

const withUnit = (v: number | null | undefined, unit: string) => (v == null ? "—" : `${v}${unit}`);

/**
 * 纵向波形柱：levels 尾部各画一根竖柱（用户 2026-11-24 点名要的柱状条）。
 * 渲染在 remoteBits 的共享 VBars（快速面板标题行复用同一份）；这里只保留
 * 「没电平摆 —」的空态口径。
 */
function VBars({ levels }: { levels: number[] }) {
  if (levels.length === 0) {
    return <div className="grid h-16 place-items-center text-sm text-ink-4">—</div>;
  }
  return <BitsVBars levels={levels} className="h-10" />;
}

/** 路由分段（radiogroup）：本地转写 / 网关转写，紧凑一行。
 *  「关闭」入口已删（用户 2026-11-24 拍板：语音不许关）——档案有语音通道就常驻可配。 */
function RoutePick({
  isLocal,
  onPick,
  disabled = false,
}: {
  isLocal: boolean;
  onPick: (r: "local" | "gateway") => void;
  /** 锁定态（用户 2026-11-25）：锁住就不许改路由——整组字段都锁了，路由能切等于没锁。 */
  disabled?: boolean;
}) {
  const items = [
    { id: "local", label: t("remote.voice.local"), on: isLocal },
    { id: "gateway", label: t("remote.voice.gateway"), on: !isLocal },
  ] as const;
  return (
    <div role="radiogroup" aria-label={t("remote.voice.route")} className="flex shrink-0 gap-1.5">
      {items.map((it) => (
        <button
          key={it.id}
          type="button"
          role="radio"
          aria-checked={it.on}
          disabled={disabled}
          onClick={() => onPick(it.id)}
          // 禁用态用实色灰字（不用 opacity 压暗，双主题对比度会崩）；当前那档留淡底让人看得出现在走哪条。
          className={`rounded-md px-2 py-1 text-sm transition-colors ${
            disabled
              ? `cursor-not-allowed text-ink-4${it.on ? " bg-[var(--key-hl)]/10" : ""}`
              : it.on
                ? "bg-[var(--key-hl)]/15 text-[var(--key-hl-text)]"
                : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
          }`}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}

/**
 * 语音坞：映射页右框的内联语音配置（替代原居中对话框）。
 * 上 = 状态 + 纵向波形柱；中 = 路由分段；下 = 当前路的字段（设备 / 输入法 / 增益 / 模型）。
 */
export function VoiceDock({
  bare,
  profileId,
  onStateChange,
}: {
  /** 嵌进三页签坞时去自带虚线框（坞已给外框）。 */
  bare?: boolean;
  /** 当前遥控器档案：增益按设备单独调、单独存。 */
  profileId?: string | null;
  /** 语音是否在采集（非 idle）上报给坞：状态驱动页签跳转。 */
  onStateChange?: (active: boolean) => void;
} = {}) {
  const qc = useQueryClient();
  const toast = useToast();
  const q = useQuery({
    queryKey: ["remote-voice"],
    queryFn: () => ipc<Snapshot>("remote_voice_get"),
  });
  // 事件比回包快：会话中的状态/错误先由事件驱动，快照随后兜底。
  const [live, setLive] = useState<{ state: VoiceState; note: string | null }>({
    state: "idle",
    note: null,
  });
  // 采集中的实时电平（0…1）。松手/失败的状态事件一到就清空：留着一条不动的曲线
  // 看着像还在收，那是假状态。
  const [levels, setLevels] = useState<number[]>([]);
  // 乐观覆盖：保存失败即回退快照值，绝不留「页面说了、后端没存」的假状态。
  const [draft, setDraft] = useState<Settings | null>(null);
  // 编辑锁（用户 2026-11-25）：默认锁住，开锁才能改下面的语音设置；选择持久化。
  const [unlocked, setUnlocked] = useState(() => {
    try {
      return localStorage.getItem(LOCK_KEY) === "1";
    } catch {
      return false;
    }
  });
  const setLock = (v: boolean) => {
    setUnlocked(v);
    try {
      localStorage.setItem(LOCK_KEY, v ? "1" : "0");
    } catch {
      /* 隐私模式存不进去，本次会话内照样生效 */
    }
  };
  // 本地卡（虚拟声卡/系统转写）组合框记住上次选择：关路再开回到上次那路。
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
      if (typeof e.partial === "string") return;
      setLevels([]);
      setLive({ state: e.state ?? "idle", note: e.error ?? null });
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

  // 增益按档案取：单独调过用单独值；1 代灵敏度低，缺省 250%（与后端同口径）。
  const gainProfile = profileId || "xiaomi_rc00x";
  const gain = settings
    ? (settings.gain_by_profile?.[gainProfile] ??
      (gainProfile === "xiaomi_gen1" ? 250 : settings.gain))
    : 100;
  const setGain = (v: number) => {
    if (!settings) return;
    const clamped = Math.min(1000, Math.max(0, v));
    patch({
      gain: clamped,
      gain_by_profile: { ...(settings.gain_by_profile ?? {}), [gainProfile]: clamped },
    });
  };
  const gainField = (
    <FieldLabel row label={t("remote.voice.gain")} hint={t("remote.voice.gainHint")}>
      <input
        aria-label={t("remote.voice.gain")}
        className="min-w-0 flex-1 accent-[var(--key-hl)]"
        type="range"
        min={0}
        max={1000}
        step={10}
        value={gain}
        onChange={(e) => setGain(Number(e.target.value))}
      />
      <span className="w-14 shrink-0 text-right text-sm tabular-nums text-ink-1">{gain}%</span>
    </FieldLabel>
  );

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
  const imes = useQuery({
    queryKey: ["remote-ime"],
    queryFn: () => ipc<ImeSource[]>("remote_ime_list"),
  });
  const imeName = imes.data?.find((i) => i.id === settings?.ime_source)?.name ?? null;

  // 事件驱动为主、快照兜底：页面中途打开时开场事件早就错过了。
  const state: VoiceState = live.state !== "idle" ? live.state : (snap?.state ?? "idle");
  // 采集态上报坞（页签自动跳转的驱动源）。
  useEffect(() => onStateChange?.(state !== "idle"), [state, onStateChange]);
  const st = STATE[state];
  const devices = snap?.devices ?? { devices: [], default: "" };
  const note = live.note ?? snap?.note ?? null;
  const m = snap?.metrics ?? null;

  return (
    // 语音坞不许滚（用户 2026-11-25）：整页不竖滚，面板自己再开滚动就把标题行顶出视野；
    // 改成 overflow-hidden + 收紧波形与行距，让内容整块装得下。
    <div className={`${bare ? "flex min-h-0 flex-1 flex-col gap-2 overflow-hidden" : `${paneCls} overflow-y-auto`}`}>
      {/* 头：话筒 + 标题 + 状态灯 + 当前电平（行高与左侧动作面板头部行对齐，用户 2026-11-25） */}
      <div className="flex h-9 shrink-0 items-center gap-2">
        <IconMic className="h-4 w-4 shrink-0 text-ink-4" />
        <span className="text-sm font-semibold text-ink-1">{t("remote.voice.title")}</span>
        <span className="ml-auto flex items-center gap-1.5 text-sm text-ink-2">
          <span aria-hidden="true">
            <StatusDot state={st.dot} busy={st.busy} text={t(st.key)} />
          </span>
          {t(st.key)}
        </span>
        <span className="w-10 shrink-0 text-right text-sm tabular-nums text-ink-4" title={t("remote.voice.levelHint")}>
          {levels.length ? `${Math.round(levels[levels.length - 1] * 100)}%` : "—"}
        </span>
      </div>
      {q.isPending && <SkeletonRows rows={2} />}
      {q.isError && (
        <QueryError inline title={t("remote.voice.loadFail")} error={q.error} onRetry={() => void q.refetch()} />
      )}
      {/* 纵向波形柱（用户 2026-11-24 点名）：平时就在，采音时实时跳动 */}
      <VBars levels={levels} />
      {settings && note && (
        <p className="text-sm leading-relaxed text-[var(--sem-danger-text)]" role="alert">
          {note}
        </p>
      )}
      {settings && (
        <>
          {/* 「关闭」入口已删（用户 2026-11-24 拍板：语音不许关）：后端存着 off 时
              分段按本地转写点亮，点任一分段即写入真实路由。
              右侧编辑锁（用户 2026-11-25）：默认锁住，开锁才能改下面的语音设置。 */}
          <div className="flex items-center gap-2">
            <RoutePick
              isLocal={isLocal || settings.route === "off"}
              disabled={!unlocked}
              onPick={(r) => {
                if (r === "local") {
                  if (!isLocal) patch({ route: localSel });
                } else patch({ route: "gateway" });
              }}
            />
            <span className="ml-auto flex shrink-0 items-center gap-1.5">
              <IconLock className={`h-3.5 w-3.5 ${unlocked ? "text-ink-4" : "text-[var(--sem-busy-text)]"}`} aria-hidden="true" />
              <Toggle
                checked={unlocked}
                onChange={setLock}
              />
              <span className="text-sm text-ink-3">{t(unlocked ? "remote.voice.unlocked" : "remote.voice.locked")}</span>
            </span>
          </div>
          {/* 锁住时整组字段禁用（原生 fieldset 一处生效，不必逐个控件传 disabled；
              display:contents 不额外占盒，禁用仍按 DOM 关系传播）。 */}
          <fieldset disabled={!unlocked} className="contents">
          {isLocal && (
            <div className="space-y-2">
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
              {localSel === "soundcard" ? (
                <>
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
                  {imeName ? (
                    <p className="text-sm text-[var(--sem-ok-text)]">{ts("remote.voice.imeReady", { name: imeName })}</p>
                  ) : (
                    <p className="text-sm text-[var(--sem-busy-text)]">{t("remote.voice.imeMissing")}</p>
                  )}
                </>
              ) : (
                <p className="text-sm leading-relaxed text-ink-4">{t("remote.voice.systemHint")}</p>
              )}
              {gainField}
            </div>
          )}
          {settings.route === "gateway" && (
            <div className="space-y-2">
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
                    {asrModels.map((mm) => (
                      <option key={mm} value={mm}>
                        {mm}
                      </option>
                    ))}
                  </select>
                )}
              </FieldLabel>
              {gainField}
            </div>
          )}
          </fieldset>
          {/* 上次会话实测指标：小字两列，没跑过会话一律 —。 */}
          {settings.route !== "off" && (
            <dl className="grid grid-cols-2 gap-x-3 gap-y-0.5 border-t border-slate-800 pt-2 text-2xs">
              <span className="flex items-baseline justify-between gap-1.5">
                <dt className="text-ink-4">{t("remote.voice.mMs")}</dt>
                <dd className="tabular-nums text-ink-1">{withUnit(m?.ms, " ms")}</dd>
              </span>
              <span className="flex items-baseline justify-between gap-1.5">
                <dt className="text-ink-4">{t("remote.voice.mRate")}</dt>
                <dd className="tabular-nums text-ink-1">{withUnit(m?.rate, " Hz")}</dd>
              </span>
              <span className="flex items-baseline justify-between gap-1.5">
                <dt className="text-ink-4">{t("remote.voice.mFrames")}</dt>
                <dd className="tabular-nums text-ink-1">{m ? String(m.frames) : "—"}</dd>
              </span>
              <span className="flex items-baseline justify-between gap-1.5">
                <dt className="text-ink-4">{t("remote.voice.mCoverage")}</dt>
                <dd className="tabular-nums text-ink-1">{m?.coverage == null ? "—" : `${m.coverage.toFixed(1)}%`}</dd>
              </span>
            </dl>
          )}
        </>
      )}
    </div>
  );
}

// 遥控器页（计划 2026-11-19-遥控器桥接重写；界面按 docs/ui_02 原型回灌）。
//
// S1 接 BLE 身份与连接：扫描/发现/连接/断开、绑定卡；S4 按键映射（真实数据骨架）在 remote/Mapping.tsx。
// 语音出口、外部服务（SayAll）的后端在 S2/S3，未接入前一律不画——
// 原型里那些区块是演示态，照搬就是造假状态（AGENTS.md 硬约束）。
// 非 macOS：整页「平台不支持」空态，不发任何 IPC（计划 §五口径）。
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isMac } from "../lib/key";
import { t } from "../lib/i18n";
import { ipc, ipcEvent } from "../lib/ipc";

import { EmptyState } from "../components/States";
import { Button, Card, SkeletonRows } from "../components/ui";
import { QueryError, errText } from "../components/QueryError";
import { useToast } from "../components/Toast";
import { IconBluetooth, IconMouse, IconRemote } from "../components/icons";
import { Mapping } from "./remote/Mapping";
import { RemoteConn } from "./remote/remoteBits";
import { hasRemoteImage } from "./remote/keys";

interface RemoteDevice {
  id: string;
  name: string | null;
  model: string | null;
  connected: boolean;
  /** 档案 id（profile.rs 唯一真相）；页面只透传，不抄档案表。 */
  profile: string | null;
}
interface RemoteStatus {
  bound: RemoteDevice | null;
  candidates: RemoteDevice[];
  ble: string;
  /** 电量 0-100；未连接/读不到 = null（页面显「—」，不造假数字）。 */
  battery: number | null;
  /** 绑定档案键表（RemoteButton id）；空 = 按键未落地（未验证档案，队列 22）。 */
  profile_keys: string[];
  /** 档案真机验过 = false 时挂「未验证」徽标，不宣称支持。 */
  profile_verified: boolean;
  /** 档案有语音通道（voice trigger 非 None）；没有就不画语音面板。 */
  profile_voice: boolean;
}

export function RemotePage() {
  const toast = useToast();
  const qc = useQueryClient();
  const status = useQuery({
    queryKey: ["remote-status"],
    queryFn: () => ipc<RemoteStatus>("remote_status"),
    enabled: isMac,
  });
  const fail = (what: string) => (e: unknown) => toast.err(`${what}：${errText(e)}`);
  const refresh = () => void qc.invalidateQueries({ queryKey: ["remote-status"] });
  // 切换/断开设备 = 档案变了：映射、面板表、方案、宏、App 绑定全按档案分仓。
  // 只刷状态会让页面挂着上一台的表（看着「乱跳」），此时编辑还会把旧表写进新档案（串档）。
  const refreshAll = () => {
    void qc.invalidateQueries({ queryKey: ["remote"] });
    void qc.invalidateQueries({ queryKey: ["quickpanel-config"] });
  };

  // 双设备同连：后端在按键时把最后使用的档案写进绑定并发这个事件；
  // 页面据此重取状态，实物图、键表、映射表和语音都自动跟过去，不需要手动切换。
  useEffect(() => {
    let off = () => {};
    void ipcEvent<{ profile: string }>("remote://active-profile", (ev) => {
      if (!ev?.profile || ev.profile === status.data?.bound?.profile) return;
      void qc.invalidateQueries({ queryKey: ["remote-status"] });
      void qc.invalidateQueries({ queryKey: ["remote"] });
      void qc.invalidateQueries({ queryKey: ["remote-macros"] });
      void qc.invalidateQueries({ queryKey: ["quickpanel-config"] });
    }).then((unsubscribe) => (off = unsubscribe));
    return () => off();
  }, [qc, status.data?.bound?.profile]);

  // TV 长按切换鼠标模式：后端发事件，页面刷新按钮与角标。
  useEffect(() => {
    let off = () => {};
    void ipcEvent<{ profile: string; enabled: boolean }>("remote://mouse-mode", () =>
      qc.invalidateQueries({ queryKey: ["remote-mouse-mode"] }),
    ).then((unsubscribe) => (off = unsubscribe));
    return () => off();
  }, [qc]);

  const scan = useMutation({
    mutationFn: () => ipc<RemoteDevice[]>("remote_scan"),
    onSuccess: (list) => {
      refresh();
      // 已绑定却扫不到候选是常态（绑定的遥控器不进候选列表），别让它读起来像设备丢了。
      toast.ok(
        list.length
          ? t("remote.scanHit").replace("{n}", String(list.length))
          : bound
            ? t("remote.scanEmptyBound")
            : t("remote.scanEmpty"),
      );
    },
    onError: fail(t("remote.scanFail")),
  });
  const connect = useMutation({
    mutationFn: (id: string) => ipc<RemoteDevice>("remote_connect", { id }),
    onSuccess: (d) => {
      refreshAll();
      toast.ok(d.model ? `${t("remote.connected")} ${d.model}` : t("remote.connectedNoModel"));
    },
    onError: fail(t("remote.connectFail")),
  });
  const disconnect = useMutation({
    mutationFn: () => ipc("remote_disconnect"),
    onSuccess: () => {
      refreshAll();
      toast.ok(t("remote.disconnected"));
    },
    onError: fail(t("remote.disconnectFail")),
  });

  // 鼠标模式按档案存 settings：默认关，关掉就恢复原来的方向键/OK/音量映射。
  // Hook 必须在 isMac 的早退之前调用，顺序才稳定。
  const mouseProfile = status.data?.bound?.profile;
  const mouseMode = useQuery({
    queryKey: ["remote-mouse-mode", mouseProfile],
    queryFn: () => ipc<boolean>("remote_mouse_mode_get", { profile: mouseProfile }),
    enabled: !!mouseProfile,
  });
  const setMouseMode = useMutation({
    mutationFn: (enabled: boolean) =>
      ipc("remote_mouse_mode_set", { profile: mouseProfile, enabled }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["remote-mouse-mode"] }),
    onError: fail(t("remote.statusFail")),
  });
  const autoHide = useQuery({
    queryKey: ["prompt-autohide"],
    queryFn: () => ipc<number>("prompt_autohide_get"),
  });
  const setAutoHide = useMutation({
    mutationFn: (seconds: number) => ipc("prompt_autohide_set", { seconds }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["prompt-autohide"] }),
    onError: fail(t("remote.statusFail")),
  });

  if (!isMac) {
    return (
      <div className="space-y-4 pt-[var(--content-pad-top)]">
        <EmptyState icon={IconRemote} title={t("remote.unsupported")} desc={t("remote.unsupportedDesc")} />
      </div>
    );
  }

  const st = status.data;
  const bound = st?.bound ?? null;
  const live = !!bound?.connected;
  const scanning = scan.isPending;
  const candidates = (st?.candidates ?? []).filter((c) => c.id !== bound?.id);

  const scanBtn = (
    <Button size="sm" onClick={() => scan.mutate()} disabled={scanning}>
      <IconBluetooth className="mr-1 inline h-3.5 w-3.5" />
      {scanning ? t("remote.scanning") : t("remote.scan")}
    </Button>
  );

  // 设备状态条（用户 2026-11-20 拍板）：绑定后并进「按键映射」卡头右侧，整页压到一屏；
  // 未绑定没卡可进，留在页顶独立一行。「状态」标题上顶栏胶囊（RemoteChip），不单独占卡。
  const statusBar = bound && (
    <>
      <span className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-sm">
        {/* 电量 + 连接状态（用户 2026-11-20 拍板口径，渲染在 remoteBits 共享件，
            快速面板标题行复用同一份）。 */}
        <RemoteConn battery={st?.battery ?? null} live={live} />
        {/* 当前设备型号：双设备同连时随最后按下的遥控器变化，一眼知道在控哪只。 */}
        {bound.model && <span className="text-2xs text-ink-3">{bound.model}</span>}
        {mouseMode.data && (
          <span className="flex items-center gap-1 text-2xs text-ink-3">
            <IconMouse className="h-3.5 w-3.5" aria-hidden />
            {t("remote.mouseMode")}
          </span>
        )}
        {/* 断开不是丢设备：macOS 会自动把 HID 链路接回来，说破省得用户以为坏了。 */}
        {!live && <span className="text-2xs text-ink-4">{t("remote.autoReconnectHint")}</span>}
        {/* 能力灯按档案（队列 22）：实心点 = 已落地、空心圈 = 未落地；文字常驻，不只靠颜色。 */}
        {st && (
          <span className="flex items-center gap-2">
            {[
              { on: st.profile_keys.length > 0, label: t("remote.capKeys") },
              { on: st.profile_voice, label: t("remote.capVoice") },
            ].map((c) => (
              <span
                key={c.label}
                className={`flex items-center gap-1 ${c.on ? "text-[var(--sem-ok-text)]" : "text-ink-4"}`}
              >
                <span
                  aria-hidden
                  className={`h-1.5 w-1.5 rounded-full ${c.on ? "bg-[var(--sem-ok)]" : "border border-slate-500"}`}
                />
                {c.label}
              </span>
            ))}
            {!st.profile_verified && (
              <span className="rounded border border-[var(--sem-busy)]/40 bg-[var(--sem-busy)]/10 px-1 text-2xs text-[var(--sem-busy-text)]">
                {t("remote.unverified")}
              </span>
            )}
          </span>
        )}
      </span>
      {live ? (
        <Button size="sm" onClick={() => disconnect.mutate()} disabled={disconnect.isPending}>
          {t("remote.disconnect")}
        </Button>
      ) : (
        scanBtn
      )}
    </>
  );

  // 设备切换组合框（用户 2026-11-21 拍板）：取代旧「候选设备 …… 连接」整行。
  // 选项 = 当前这台 + 候选；选候选即 remote_connect——绑定跟着走，
  // 下方实物图 / 映射卡 / 虚拟键盘实物图全部随 status.bound 切换。
  const devLabel = (d: RemoteDevice) => [d.name, d.model].filter(Boolean).join(" ") || d.id;
  const deviceSelect = (
    <span className="flex min-w-0 items-center gap-1">
      <select
        aria-label={t("remote.deviceSwitch")}
        data-tip={t("remote.deviceSwitch")}
        className="tip max-w-52 truncate rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-sm font-normal normal-case tracking-normal text-ink-3"
        value={bound?.id ?? ""}
        disabled={connect.isPending}
        onChange={(e) => {
          const id = e.target.value;
          if (id && id !== bound?.id) connect.mutate(id);
        }}
      >
        {!bound && (
          <option value="" disabled>
            {t("remote.devicePick")}
          </option>
        )}
        {bound && <option value={bound.id}>{devLabel(bound)}</option>}
        {candidates.map((c) => (
          <option key={c.id} value={c.id}>
            {devLabel(c)}
          </option>
        ))}
      </select>
      <Button
        size="sm"
        onClick={() => scan.mutate()}
        disabled={scanning}
        data-tip={t("remote.scan")}
        aria-label={t("remote.scan")}
      >
        <IconBluetooth className={`h-3.5 w-3.5 ${scanning ? "animate-spin" : ""}`} />
      </Button>
      {mouseProfile && (
        <Button
          size="sm"
          aria-pressed={!!mouseMode.data}
          data-tip={t("remote.mouseModeTip")}
          onClick={() => setMouseMode.mutate(!mouseMode.data)}
          className={mouseMode.data ? "text-[var(--key-hl-text)]" : ""}
        >
          <IconMouse className="mr-1 inline h-3.5 w-3.5" />
          {t("remote.mouseMode")} {mouseMode.data ? t("remote.mouseOn") : t("remote.mouseOff")}
        </Button>
      )}
      <label className="flex items-center gap-1 text-sm text-ink-3" title={t("remote.autoHideTip")}>
        {t("remote.autoHide")}
        <input
          className="w-16 rounded-md border border-slate-700 bg-transparent px-1.5 py-0.5 text-sm tabular-nums"
          type="number"
          min={0}
          max={300}
          value={autoHide.data ?? 10}
          aria-label={t("remote.autoHide")}
          onChange={(e) =>
            setAutoHide.mutate(Math.min(300, Math.max(0, Number(e.target.value) || 0)))
          }
        />
      </label>
    </span>
  );

  // 卡头控件（用户 2026-11-24 拍板）：正常在场不挂设备组合框，型号悬停在卡头遥控器图标上；
  // 只有场上还有别的候选设备时才把切换器请出来——否则连接中根本没法换设备。
  const headControls = candidates.length > 0 ? deviceSelect : (
    <span className="flex min-w-0 items-center gap-1">
      <Button
        size="sm"
        onClick={() => scan.mutate()}
        disabled={scanning}
        data-tip={t("remote.scan")}
        aria-label={t("remote.scan")}
      >
        <IconBluetooth className={`h-3.5 w-3.5 ${scanning ? "animate-spin" : ""}`} />
      </Button>
    </span>
  );
  const deviceTip = bound ? [bound.name, bound.model].filter(Boolean).join(" ") || bound.id : undefined;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 pt-[var(--content-pad-top)]">
      {/* 状态未落地先出骨架（审查 2.1）：回包前 !bound 成立，已绑定用户会闪一下空态。 */}
      {status.isPending && (
        <div className="space-y-3">
          <SkeletonRows rows={3} />
        </div>
      )}

      {status.isError && (
        <QueryError
          inline
          title={t("remote.statusFail")}
          error={status.error}
          onRetry={() => void status.refetch()}
        />
      )}

      {/* 未绑定：提示一行 + 主 CTA 归空态卡（审查 1.5）——同屏两个「扫描遥控器」实心按钮是视觉重心分裂。 */}
      {!status.isPending && !bound && (
        <p className="border-b border-slate-800 pb-2 text-sm text-ink-4">{t("remote.bleHint")}</p>
      )}

      {/* 扫出候选（或正在扫）时空态卡收起，候选列表独占这块（审查 1.5）。 */}
      {!status.isPending && !bound && candidates.length === 0 && !scanning && (
        <EmptyState
          icon={IconBluetooth}
          title={t("remote.noneTitle")}
          desc={t("remote.noneDesc")}
          action={
            <span className="flex gap-2">
              <Button onClick={() => scan.mutate()} disabled={scanning}>
                {scanning ? t("remote.scanning") : t("remote.scan")}
              </Button>
            </span>
          }
        />
      )}

      {/* 未绑定且有候选（或正在扫）：组合框行接管旧「候选设备」行（选候选=连接）。 */}
      {!status.isPending && !bound && (candidates.length > 0 || scanning) && (
        <div className="flex items-center gap-2 border-b border-slate-800 pb-2">{deviceSelect}</div>
      )}

      {/* 按键映射区：绑定过就显示（配置落 settings，未连接也能改，连上即生效）。
          实物图在这里当叠放底图。档案键表为空的（未验证档案）不拿小米键位表冒充，
          改「未验证」状态卡说清楚（队列 22）。 */}
      {bound && st && st.profile_keys.length === 0 && (
        <>
          {/* 状态条原本并进 Mapping 卡头；未验证档案没有映射卡，状态条独立一行，
              电量/断开入口不许跟着消失；组合框同排——不然卡在未验证页切不回别的设备。 */}
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-slate-800 pb-2">
            {deviceSelect}
            <span className="ml-auto flex flex-wrap items-center gap-x-4 gap-y-1">{statusBar}</span>
          </div>
          <Card>
            <div className="flex items-start gap-2.5">
              <IconRemote className="mt-0.5 h-4 w-4 shrink-0 text-ink-4" />
              <div>
                <p className="text-sm font-medium text-ink-1">{t("remote.unverifiedTitle")}</p>
                <p className="mt-1 text-sm text-ink-4">{t("remote.unverifiedDesc")}</p>
              </div>
            </div>
          </Card>
        </>
      )}
      {bound && st && st.profile_keys.length > 0 && (
        // min-h-0 是整条内部滚动链的总闸（用户 2026-11-24 两次抓撑高）：
        // 外壳 section 是 overflow-auto 的 flex 列，这里缺 min-h-0 时 flex 子项
        // 默认 min-height:auto 不许收缩，动作列表一长整页就被撑出去滚动，
        // 面板内部的 overflow-y-auto 永远轮不到生效。
        <div className="flex min-h-0 flex-1 flex-col">
          <Mapping
            statusBar={statusBar}
            deviceSelect={headControls}
            deviceTip={deviceTip}
            profileKeys={st.profile_keys}
            hasImage={hasRemoteImage(bound.profile)}
            profileId={bound.profile}
            voiceAvailable={!!st.profile_voice}
          />
        </div>
      )}

    </div>
  );
}

// 遥控器页（计划 2026-11-19-遥控器桥接重写；界面按 docs/ui_02 原型回灌）。
//
// S1 接 BLE 身份与连接：扫描/发现/连接/断开、绑定卡；S4 按键映射（真实数据骨架）在 remote/Mapping.tsx。
// 语音出口、外部服务（SayAll）的后端在 S2/S3，未接入前一律不画——
// 原型里那些区块是演示态，照搬就是造假状态（AGENTS.md 硬约束）。
// 非 macOS：整页「平台不支持」空态，不发任何 IPC（计划 §五口径）。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isMac } from "../lib/key";
import { t } from "../lib/i18n";
import { ipc } from "../lib/ipc";

import { EmptyState } from "../components/States";
import { Button, Card, SkeletonRows } from "../components/ui";
import { QueryError, errText } from "../components/QueryError";
import { useToast } from "../components/Toast";
import { IconBluetooth, IconBattery, IconRemote } from "../components/icons";
import { Mapping } from "./remote/Mapping";
import { VoicePanel } from "./remote/VoicePanel";
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
      refresh();
      toast.ok(d.model ? `${t("remote.connected")} ${d.model}` : t("remote.connectedNoModel"));
    },
    onError: fail(t("remote.connectFail")),
  });
  const disconnect = useMutation({
    mutationFn: () => ipc("remote_disconnect"),
    onSuccess: () => {
      refresh();
      toast.ok(t("remote.disconnected"));
    },
    onError: fail(t("remote.disconnectFail")),
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
        {/* 电量（用户 2026-11-20 拍板）：横置电池图标 + 百分比，≤20% 红、其余荧光绿；读不到显 —。
            「型号」条目已删（用户 2026-11-21）：与设备组合框重复，标识码 tooltip 一并去掉。 */}
        <span
          className="tip flex items-center gap-1.5"
          data-tip={t("remote.battery")}
          aria-label={`${t("remote.battery")} ${st?.battery != null ? `${st.battery}%` : "—"}`}
        >
          {st?.battery != null ? (
            <b
              className={`flex items-center gap-1 ${
                st.battery <= 20 ? "text-[var(--sem-danger-text)]" : "text-[var(--sem-ok-text)]"
              }`}
            >
              <IconBattery level={st.battery / 100} className="h-5 w-9" />
              {st.battery}%
            </b>
          ) : (
            <b className="text-ink-1">—</b>
          )}
        </span>
        <span className="text-ink-3">
          {t("remote.connState")}{" "}
          <b className={live ? "text-[var(--sem-ok-text)]" : "text-ink-4"}>
            {live ? t("remote.connected") : t("remote.boundIdle")}
          </b>
        </span>
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
              <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1 text-2xs text-[var(--sem-busy-text)]">
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
    </span>
  );

  return (
    <div className="space-y-4 pt-[var(--content-pad-top)]">
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
        <Mapping
          statusBar={statusBar}
          deviceSelect={deviceSelect}
          profileKeys={st.profile_keys}
          hasImage={hasRemoteImage(bound.profile)}
        />
      )}
      {/* 语音面板按档案能力摆：档案没有语音通道就不画（不造假状态）。 */}
      {bound && st?.profile_voice && <VoicePanel />}
    </div>
  );
}

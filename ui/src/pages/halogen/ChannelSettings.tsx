// 机位设置（halogen / gufo 共用一份）：SSH 地址、用户名、部署目录。
//
// 为什么要有这个界面：出厂默认写的是开发者那台机器（雷电直连 + 局域网兜底、
// 固定部署目录），但用户机器上的内网、地址、目录都可能不一样。以前只能手编
// config.yaml，改错了还看不出错在哪；这里把同一件事收进界面，并如实标出
// 「与出厂默认不同」。改动落 settings 覆盖层，重启后依旧生效，不碰 config.yaml。
import { useEffect, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, FieldLabel, fieldCls } from "../../components/ui";
import { useToast } from "../../components/Toast";
import { ipc } from "../../lib/ipc";
import { t, tf, ts } from "../../lib/i18n";

interface HalctlCfg {
  hosts: string[];
  user: string;
  proj: string;
}

interface CfgView {
  ch: string;
  effective: HalctlCfg;
  default: HalctlCfg;
  overridden: boolean;
}

export function useHalChannelCfg(ch: string) {
  return useQuery({
    queryKey: ["hal", ch, "cfg"],
    queryFn: () => ipc<CfgView>("halctl_cfg_get", { ch }),
    staleTime: 30_000,
  });
}

/** 当前链路在配置里的位置：首选 / 备用 N。不再按字面 IP 猜「雷电/局域网」。 */
export function linkLabel(host: string, hosts: string[]): string {
  const i = hosts.indexOf(host);
  if (i < 0) return ts("hl.link", { host });
  return i === 0 ? ts("hl.linkPrimary", { host }) : ts("hl.linkBackup", { n: i, host });
}

export function ChannelSettings({ ch, label }: { ch: string; label: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const cfg = useHalChannelCfg(ch);
  const [hosts, setHosts] = useState("");
  const [user, setUser] = useState("");
  const [proj, setProj] = useState("");

  // 每次打开都从服务端生效值回填，避免拿着过期草稿改。
  useEffect(() => {
    if (!open || !cfg.data) return;
    const e = cfg.data.effective;
    setHosts(e.hosts.join("\n"));
    setUser(e.user);
    setProj(e.proj);
  }, [open, cfg.data]);

  const save = useMutation({
    mutationFn: () =>
      ipc<CfgView>("halctl_cfg_set", {
        ch,
        hosts: hosts.split("\n").map((s) => s.trim()).filter(Boolean),
        user,
        proj,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["hal", ch] });
      toast.ok(ts("hl.cfgSaved", { label }));
      setOpen(false);
    },
    onError: (e) => toast.err(ts("dlg.saveFail", { e: String(e) })),
  });

  const d = cfg.data;
  const differs = (a: string, b: string) => a !== b;

  return (
    <>
      <Button variant="ghost" onClick={() => setOpen(true)}>
        {t("hl.cfgBtn")}
      </Button>
      <Dialog.Root open={open} onOpenChange={setOpen}>
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[520px] max-w-[94vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <Dialog.Title className="text-base font-semibold text-ink-1">
            {ts("hl.cfgTitle", { label })}
          </Dialog.Title>
          <Dialog.Description className="mt-1 text-xs text-ink-3">
            {t("hl.cfgDesc")}
          </Dialog.Description>

          {cfg.isError && (
            <p className="mt-3 text-xs text-[var(--sem-danger-text)]">
              {ts("hl.cfgReadFail", { e: String(cfg.error) })}
            </p>
          )}

          <div className="mt-4 space-y-3">
            <FieldLabel label={t("hl.fHosts")}>
              <textarea
                className={`${fieldCls} min-h-20 font-mono text-xs`}
                value={hosts}
                onChange={(e) => setHosts(e.target.value)}
                placeholder={"10.0.0.102\n192.168.31.168"}
              />
              {d && differs(d.effective.hosts.join(","), d.default.hosts.join(",")) && (
                <p className="text-2xs text-[var(--sem-busy-text)]">
                  {ts("hl.fDefault", { hosts: d.default.hosts.join(" → ") })}
                </p>
              )}
            </FieldLabel>
            <div className="grid grid-cols-2 gap-3">
              <FieldLabel label={t("hl.fUser")}>
                <input className={fieldCls} value={user} onChange={(e) => setUser(e.target.value)} />
              </FieldLabel>
              <FieldLabel label={t("hl.fProj")}>
                <input
                  className={`${fieldCls} font-mono text-xs`}
                  value={proj}
                  onChange={(e) => setProj(e.target.value)}
                />
              </FieldLabel>
            </div>
            {d?.overridden && (
              <p className="text-2xs text-ink-4">
                {tf("hl.overrideNote", {
                  c: <code>config.yaml</code>,
                  s: <code>{ch === "gufo" ? "guctl" : "halctl"}</code>,
                })}
              </p>
            )}
          </div>

          <div className="mt-5 flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setOpen(false)}>
              {t("dlg.cancel")}
            </Button>
            <Button variant="primary" disabled={save.isPending} onClick={() => void save.mutate()}>
              {save.isPending ? t("save.saving") : t("dlg.save")}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Root>
    </>
  );
}

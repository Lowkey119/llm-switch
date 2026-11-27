// 后端新增/编辑弹窗（计划 §6.1 自 Backends.tsx 拆出，行为不变）。
import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { t, ts } from "../../lib/i18n";
import { Button, inputLgCls } from "../../components/ui";
import { Field } from "../../components/Field";
import { ModelPicker, probeTargetKey, type ProbeSnapshot } from "../../components/ModelPicker";
import { ipc, type BackendInput, type BackendRow } from "../../lib/ipc";

function seed(row: BackendRow | null | undefined, blank: BackendInput): BackendInput {
  if (!row) return blank;
  return {
    ...blank,
    ...row,
    api_key: row.api_key ?? "",
    webui_password: row.webui_password ?? "",
    models: [...row.models],
  };
}

export function BackendDialog({
  open,
  initial,
  onClose,
  onDone,
}: {
  open: boolean;
  /** 传了就是编辑模式（按 name 定位，改名等于新建，所以名称锁死）。 */
  initial?: BackendRow | null;
  onClose: () => void;
  /** 落库成功后的收尾（刷新列表）；提示由本弹窗自己发。 */
  onDone: () => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const editing = !!initial;
  const blank: BackendInput = {
    name: "",
    base_url: "",
    protocol: "openai_chat",
    api_key: "",
    weight: 1,
    enabled: true,
    models: [],
    webui_url: "",
    webui_user: "",
    webui_password: "",
  };
  const stamp = `${open ? "o" : "c"}:${initial?.id ?? "new"}`;
  const [key, setKey] = useState(stamp);
  const [form, setForm] = useState<BackendInput>(() => seed(initial, blank));
  const [models, setModels] = useState<string[]>(() => (initial ? [...initial.models] : []));
  /** 扫描带回的真值快照（保存时比对指纹，连接信息变过就作废）。 */
  const [probe, setProbe] = useState<ProbeSnapshot | null>(null);

  // 对话框是常驻挂载的（Radix 进出动画要），所以用「身份变了就整体重挂一次」
  // 这种写法重新灌表单，比在渲染里 setState 干净（也躲开 react-hooks 红线）。
  if (key !== stamp) {
    setKey(stamp);
    setForm(seed(initial, blank));
    setModels(initial ? [...initial.models] : []);
    setProbe(null);
  }

  // 列表接口只回显掩码，扫描时别把一串星号当密钥发出去。
  const probeKey = form.api_key && form.api_key !== "***" ? form.api_key : undefined;

  const save = useMutation({
    mutationFn: () => {
      // 扫描真值只在「连接信息没变过」时随保存落库：换过 URL/密钥的旧快照作废，
      // 后端侧只写探测列、不动清单（拿不到的值 COALESCE 保留，清单以勾选为准）。
      const target = { base_url: form.base_url, protocol: form.protocol, api_key: probeKey };
      const snap =
        probe && probeTargetKey(probe.target) === probeTargetKey(target) ? probe.models : [];
      return ipc("upsert_backend", {
        input: {
          ...form,
          // 编辑态带上原名：不带的话后端会把「同名新增」当整行覆盖处理。
          original_name: initial?.name,
          models: models.map((m) => m.trim()).filter(Boolean),
          probe: snap,
        },
      });
    },
    onSuccess: () => {
      toast.ok(editing ? t("dlg.savedEdit") : t("dlg.savedNew"));
      onDone();
      qc.invalidateQueries({ queryKey: ["backends"] });
      onClose();
    },
    onError: (e) => toast.err(ts("dlg.saveFail", { e: String(e) })),
  });

  const field = inputLgCls;

  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        {/* 百分比限高（勿改回 vh）：documentElement 的 CSS zoom 会把 vh 乘高。 */}
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] flex max-h-[var(--dialog-max-h)] w-[480px] max-w-[92%] -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <Dialog.Title className="shrink-0 text-base font-semibold">
            {editing ? ts("dlg.editTitle", { name: initial?.name ?? "" }) : t("dlg.addTitle")}
          </Dialog.Title>
          {/* 中间区滚动、底部按钮钉住：扫出几十个模型也不会把「保存」顶出屏幕 */}
          <div className="mt-4 min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
            <Field
              label={t("dlg.name")}
              hint={editing ? t("dlg.nameEditHint") : undefined}
            >
              <input
                className={field}
                placeholder="my-backend"
                value={form.name}
                disabled={editing}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
            </Field>
            <Field label="Base URL" hint={t("dlg.baseUrlHint")}>
              <input
                className={field}
                placeholder="http://127.0.0.1:8000"
                value={form.base_url}
                onChange={(e) => setForm({ ...form, base_url: e.target.value })}
              />
            </Field>
            <div className="flex gap-2">
              <Field label={t("dlg.protocol")} className="flex-1">
                <select
                  className={field}
                  value={form.protocol}
                  onChange={(e) => setForm({ ...form, protocol: e.target.value })}
                >
                  <option value="openai_chat">openai_chat</option>
                  <option value="anthropic">anthropic</option>
                  {/* 云 ComfyUI 视频 worker（切片 B）：无聊天适配，只被 /v1/videos 任务代理消费。 */}
                  <option value="comfyui">comfyui</option>
                </select>
              </Field>
              <Field label={t("dlg.weight")} hint={t("dlg.weightHint")} className="w-28">
                <input
                  className={field}
                  type="number"
                  min={1}
                  value={form.weight}
                  onChange={(e) => setForm({ ...form, weight: Number(e.target.value) || 1 })}
                />
              </Field>
            </div>
            <Field
              label="API Key"
              hint={
                editing && form.api_key === "***"
                  ? t("dlg.keyEditHint")
                  : undefined
              }
            >
              <input
                className={field}
                placeholder="sk-…"
                value={form.api_key ?? ""}
                onChange={(e) => setForm({ ...form, api_key: e.target.value })}
              />
            </Field>
            <Field label={t("dlg.consoleUrl")} hint={t("dlg.consoleUrlHint")}>
              <input
                className={field}
                placeholder="http://127.0.0.1:8080"
                value={form.webui_url ?? ""}
                onChange={(e) => setForm({ ...form, webui_url: e.target.value })}
              />
            </Field>
            <div className="flex gap-2">
              <Field label={t("dlg.consoleUser")} className="flex-1">
                <input
                  className={field}
                  placeholder={t("dlg.optional")}
                  value={form.webui_user ?? ""}
                  onChange={(e) => setForm({ ...form, webui_user: e.target.value })}
                />
              </Field>
              <Field label={t("dlg.consolePass")} className="flex-1">
                <input
                  className={field}
                  type="password"
                  placeholder={t("dlg.optional")}
                  value={form.webui_password ?? ""}
                  onChange={(e) => setForm({ ...form, webui_password: e.target.value })}
                />
              </Field>
            </div>
            <ModelPicker
              models={models}
              onChange={setModels}
              onProbe={setProbe}
              target={() => ({
                base_url: form.base_url,
                protocol: form.protocol,
                api_key: probeKey,
              })}
            />
          </div>
          <div className="mt-5 flex shrink-0 justify-end gap-2">
            <Button onClick={onClose}>{t("dlg.cancel")}</Button>
            <Button variant="primary" onClick={() => save.mutate()} disabled={!form.name || !form.base_url}>
              保存
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

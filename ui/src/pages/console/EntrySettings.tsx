// 入口设置表单（从 ConsolePage 原样搬出）：webui 三元组 + 启停脚本。
// form 状态跟着「谁用谁持有」搬进来，预填 effect 也在这——依赖带上 webui
// 三元组与两个脚本路径：后端返回值变了表单必须跟着刷新，否则页面上是旧地址。
// refetch 拿回同样的值时这些是同值字符串，依赖不变、不会打断正在编辑的输入。
import { useEffect, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Button, Card, inputCls } from "../../components/ui";
import { useToast } from "../../components/Toast";
import { Field } from "../../components/Field";
import { ipc, type BackendRow } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";

interface Form {
  webui_url: string;
  webui_user: string;
  webui_password: string;
  start_script: string;
  stop_script: string;
}

export function EntrySettings({
  sel,
  open,
  onClose,
  onSaved,
}: {
  sel: BackendRow;
  open: boolean;
  onClose: () => void;
  /** 保存成功后的页面动作：重读 backends + 重载控制台帧。 */
  onSaved: () => void;
}) {
  const toast = useToast();
  const [form, setForm] = useState<Form>({
    webui_url: "",
    webui_user: "",
    webui_password: "",
    start_script: "",
    stop_script: "",
  });

  // 选中项变化时预填编辑表单（密码回显掩码 = 保存时原样送回即保持不变）。
  useEffect(() => {
    setForm({
      webui_url: sel.webui_url ?? "",
      webui_user: sel.webui_user ?? "",
      webui_password: sel.webui_password ?? "",
      start_script: sel.start_script ?? "",
      stop_script: sel.stop_script ?? "",
    });
  }, [
    sel.id,
    sel.webui_url,
    sel.webui_user,
    sel.webui_password,
    sel.start_script,
    sel.stop_script,
  ]);

  const save = useMutation({
    mutationFn: () =>
      ipc("set_backend_console", {
        name: sel.name,
        // Tauri v2 命令参数一律按 camelCase 收，snake_case 会被判缺参。
        webuiUrl: form.webui_url.trim(),
        webuiUser: form.webui_user.trim(),
        // 空或掩码 → 送掩码哨兵保持既有密码。
        webuiPassword: form.webui_password || "***",
        // 脚本没有掩码概念：留空就是清掉。
        startScript: form.start_script.trim(),
        stopScript: form.stop_script.trim(),
      }),
    onSuccess: () => {
      toast.ok(t("cs.entrySaved"));
      onSaved();
      onClose();
    },
    onError: (e) => toast.err(ts("cs.entrySaveFail", { e: String(e) })),
  });

  if (!open) return null;
  return (
    <Card className="mb-3 space-y-2 py-3">
      <div className="form-grid gap-2">
        <Field label={t("cs.fUrl")} hint={t("cs.fUrlHint")}>
          <input
            className={inputCls}
            placeholder="http://127.0.0.1:8080"
            value={form.webui_url}
            onChange={(e) => setForm({ ...form, webui_url: e.target.value })}
          />
        </Field>
        <Field label={t("cs.fUser")} hint={t("dlg.optional")}>
          <input
            className={inputCls}
            value={form.webui_user}
            onChange={(e) => setForm({ ...form, webui_user: e.target.value })}
          />
        </Field>
        <Field label={t("cs.fPass")} hint={sel.webui_password ? t("cs.passKept") : t("dlg.optional")}>
          <input
            className={inputCls}
            type="password"
            value={form.webui_password}
            onChange={(e) => setForm({ ...form, webui_password: e.target.value })}
          />
        </Field>
      </div>
      <div className="form-grid gap-2">
        <Field label={t("cs.fStart")} hint={t("cs.localPathOpt")}>
          <input
            className={inputCls}
            placeholder="~/llama.cpp/serve-ornith.sh"
            value={form.start_script}
            onChange={(e) => setForm({ ...form, start_script: e.target.value })}
          />
        </Field>
        <Field label={t("cs.fStop")} hint={t("cs.localPathOpt")}>
          <input
            className={inputCls}
            placeholder="~/llama.cpp/stop-ornith.sh"
            value={form.stop_script}
            onChange={(e) => setForm({ ...form, stop_script: e.target.value })}
          />
        </Field>
      </div>
      <div className="flex items-center gap-2">
        <Button variant="primary" onClick={() => save.mutate()} disabled={save.isPending}>
          {t("cs.saveReload")}
        </Button>
        <span className="text-sm text-ink-4">{t("cs.authNote")}</span>
      </div>
    </Card>
  );
}

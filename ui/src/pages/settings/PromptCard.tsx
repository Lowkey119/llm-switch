// 设置页 · 提示词速取面板卡（计划 2026-11-18 P0-9）：
// 唤起快捷键自定义（组合键录入，冲突/非法由后端把关并回人话错误）+
// macOS 辅助功能授权引导（模拟粘贴的前置条件，无授权时注入静默无效）。
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, CardHead } from "../../components/ui";
import { ipc } from "../../lib/ipc";
import { t } from "../../lib/i18n";
import { isMac } from "../../lib/key";

/** 展示用美化：修饰键换符号（mac 用系统符号，其余用缩写）。 */
function prettyAccel(accel: string): string {
  const map: Record<string, string> = isMac
    ? { Command: "⌘", Control: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" }
    : { Command: "Win", Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win" };
  return accel
    .split("+")
    .map((p) => map[p] ?? p)
    .join(isMac ? "" : "+");
}

/** 从按键事件拼 tauri accelerator 串；纯修饰键返回 null（还没按完）。 */
function accelFromEvent(e: KeyboardEvent): string | null {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super"); // mac 上是 ⌘，Windows/Linux 上是系统键
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null;
  let k = e.key;
  const mCode = /^Key([A-Z])$/.exec(e.code);
  const mDigit = /^Digit(\d)$/.exec(e.code);
  const mFn = /^F(\d{1,2})$/.exec(e.code);
  if (mCode) k = mCode[1];
  else if (mDigit) k = mDigit[1];
  else if (mFn) k = e.code.toUpperCase();
  else if (k === " ") k = "Space";
  else if (k.length === 1) k = k.toUpperCase();
  else return null; // 其余（媒体键等）不是合法 accelerator 键名
  if (!mods.length) return "";
  return [...mods, k].join("+");
}

export function PromptCard() {
  const toast = useToast();
  const hotkey = useQuery({ queryKey: ["prompt-hotkey"], queryFn: () => ipc<string>("prompt_hotkey_get") });
  // 辅助功能实时探一次（只探不弹系统框）：ad-hoc 签的包重编后旧授权不算，
  // 静态提示会把人带偏，这里只报当前构建的真状态。
  const a11y = useQuery({ queryKey: ["prompt-a11y"], queryFn: () => ipc<boolean>("prompt_a11y_status") });
  const [recording, setRecording] = useState(false);

  // 录入态：捕获阶段截键，抢在 App 的 Cmd+1..8 等全局键之前。
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        return;
      }
      const accel = accelFromEvent(e);
      if (accel === null) return; // 只按了修饰键，继续等主键
      if (accel === "") {
        toast.err(t("prompt.cfgNeedMod"));
        setRecording(false);
        return;
      }
      setRecording(false);
      void ipc<string>("prompt_hotkey_set", { accelerator: accel })
        .then((v) => {
          hotkey.refetch();
          toast.ok(`${t("prompt.cfgHotkeyOk")} ${prettyAccel(v)}`);
        })
        .catch((e) => toast.err(String(e)));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // hotkey.refetch 稳定；toast/t 每次取新无妨。
  }, [recording]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <Card>
      <CardHead title={t("prompt.cfgTitle")} />
      <div className="flex flex-wrap items-center gap-3 text-sm">
        <span className="text-ink-2">{t("prompt.cfgHotkey")}</span>
        {recording ? (
          <span className="rounded-md bg-slate-800 px-2 py-1 font-mono text-2xs text-[var(--sem-busy-text)]">
            {t("prompt.cfgRecording")}
          </span>
        ) : (
          <>
            <span className="rounded-md bg-slate-800 px-2 py-1 font-mono text-2xs text-ink-1">
              {hotkey.data ? prettyAccel(hotkey.data) : "—"}
            </span>
            <Button onClick={() => setRecording(true)}>{t("prompt.cfgChange")}</Button>
          </>
        )}
      </div>
      <p className="mt-2 text-2xs text-ink-3">{t("prompt.cfgTip")}</p>
      {isMac && (
        <div className="mt-3 border-t border-slate-800 pt-3">
          <div className="flex flex-wrap items-center gap-2 text-2xs">
            {/* 状态不只靠颜色：文字自己说清已授权 / 未授权。 */}
            <span
              className={
                a11y.data === false
                  ? "text-[var(--sem-danger-text)]"
                  : a11y.data
                    ? "text-[var(--sem-ok-text)]"
                    : "text-ink-4"
              }
            >
              {a11y.data === false ? t("prompt.cfgA11yNo") : a11y.data ? t("prompt.cfgA11yOk") : "—"}
            </span>
            <Button onClick={() => void a11y.refetch()}>{t("prompt.cfgA11yCheck")}</Button>
            <Button onClick={() => void ipc("prompt_a11y_open").catch(() => {})}>
              {t("prompt.cfgA11yOpen")}
            </Button>
            <Button
              variant="ghost"
              onClick={() =>
                void ipc("prompt_a11y_reset")
                  .then(() => a11y.refetch())
                  .catch(() => {})
              }
            >
              {t("prompt.cfgA11yReset")}
            </Button>
          </div>
          <p className="mt-1.5 text-2xs text-ink-3">{t("prompt.cfgA11y")}</p>
        </div>
      )}
    </Card>
  );
}

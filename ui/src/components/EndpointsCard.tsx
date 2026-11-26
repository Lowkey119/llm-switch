// API 端点速览卡：监听地址 + 各协议入口 + 一键复制 curl。
// 协议名（OpenAI/Anthropic/Gemini）属专有名词、路径属数据，均不翻译。
// 平时折叠（用户口径）：这卡是低频参考，默认收起，点开才展开各端点，不占设置页首屏。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, Card } from "./ui";
import { IconCaret } from "./icons";
import { useToast } from "./Toast";
import { ipc, type ShellStatus } from "../lib/ipc";
import { t } from "../lib/i18n";

const ENDPOINTS: [string, string][] = [
  ["/v1/chat/completions", "OpenAI Chat"],
  ["/v1/responses", "OpenAI Responses"],
  ["/v1/messages", "Anthropic Messages"],
  ["/v1beta", "Gemini"],
  ["/v1/models", "Models"],
];

export function EndpointsCard() {
  const status = useQuery({ queryKey: ["status"], queryFn: () => ipc<ShellStatus>("get_status") });
  const toast = useToast();
  // 默认折叠：点开标题才展开监听地址与各协议入口。
  const [open, setOpen] = useState(false);
  const listen = status.data?.listen ?? "";
  const base = listen ? `http://${listen}` : "";
  const token = status.data?.access_token ?? null;

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      toast.err(t("ec.copyFail"));
    }
  };

  return (
    <Card>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 text-left"
        aria-expanded={open}
      >
        <span className="text-xs font-semibold uppercase tracking-wider text-ink-3">{t("ec.title")}</span>
        <span className="flex items-center gap-1 text-sm text-ink-4">
          <IconCaret open={open} />
          {open ? t("ec.collapse") : t("ec.expand")}
        </span>
      </button>
      {open && (
        <>
      <div className="mb-2 flex flex-wrap items-center gap-2 text-sm">
        <span className="text-ink-3">{t("ec.listen")}</span>
        <code className="break-all text-ink-1">{base || "—"}</code>
        {base && (
          <Button size="sm" onClick={() => copy(base)}>
            {t("ec.copy")}
          </Button>
        )}
      </div>
      <p className="mb-3 text-sm leading-relaxed text-ink-4">
        {token ? t("ec.tokenOn") : t("ec.tokenOff")}
      </p>

      <div className="space-y-1.5 text-sm">
        {ENDPOINTS.map(([path, label]) => {
          const curl = `curl ${base}${path} -H "Authorization: Bearer <token>"`;
          return (
            <div key={path} className="flex flex-wrap items-center gap-2">
              <code className="break-all text-ink-2">{path}</code>
              <span className="text-ink-4">{label}</span>
              <Button size="sm" onClick={() => copy(curl)}>
                {t("ec.copy")}
              </Button>
            </div>
          );
        })}
      </div>
        </>
      )}
    </Card>
  );
}

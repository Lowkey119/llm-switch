// API 端点速览卡：监听地址 + 各协议入口 + 一键复制 curl。
// 协议名（OpenAI/Anthropic/Gemini）属专有名词、路径属数据，均不翻译。
// 平时折叠（用户口径）：这卡是低频参考，默认收起，点开才展开各端点，不占设置页首屏。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, Card } from "./ui";
import { useToast } from "./Toast";
import { ipc, type ShellStatus } from "../lib/ipc";

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
      toast.err("复制失败");
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
        <span className="text-xs font-semibold uppercase tracking-wider text-ink-3">API 端点</span>
        <span className="text-xs text-ink-4">{open ? "▾ 收起" : "▸ 展开"}</span>
      </button>
      {open && (
        <>
      <div className="mb-2 flex flex-wrap items-center gap-2 text-xs">
        <span className="text-ink-3">监听地址</span>
        <code className="break-all text-ink-1">{base || "—"}</code>
        {base && (
          <Button size="sm" onClick={() => copy(base)}>
            复制
          </Button>
        )}
      </div>
      <p className="mb-3 text-xs leading-relaxed text-ink-4">
        {token
          ? "已启用访问令牌：请求须带 Authorization: Bearer <令牌>。"
          : "未设访问令牌：本机直连即可，无需鉴权头。"}
      </p>

      <div className="space-y-1.5 text-xs">
        {ENDPOINTS.map(([path, label]) => {
          const curl = `curl ${base}${path} -H "Authorization: Bearer <token>"`;
          return (
            <div key={path} className="flex flex-wrap items-center gap-2">
              <code className="break-all text-ink-2">{path}</code>
              <span className="text-ink-4">{label}</span>
              <Button size="sm" onClick={() => copy(curl)}>
                复制
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

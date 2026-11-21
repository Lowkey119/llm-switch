// JSON Tab（U-E）：Store 投影的后端配置直编。
// 自动保存（用户拍板 Q5）：输入即校验，防抖 1s 落盘；解析失败只显示错误、不发请求。
// 后端还有闸②语义校验与闸③并发指纹（别处改过会拒绝覆盖），错误原文都摊在顶部红字里。
// 密钥导出恒为 ***，导入时 *** = 保持原值——这里看不见也存不走真实密钥。
import { useEffect, useRef, useState } from "react";
import { useToast } from "./Toast";
import { IconAlert } from "./icons";
import { ipc } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

interface Exported {
  text: string;
  sha: string;
}

interface ImportResult {
  added: number;
  updated: number;
  removed: number;
  /** 被「halogen 不许删」那道闸留下的后端名——JSON 里删了它也不算数，得如实说。 */
  kept_protected?: string[];
  sha: string;
  text: string;
}

export function JsonTab() {
  const toast = useToast();
  const [text, setText] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const shaRef = useRef("");
  const lastSent = useRef("");
  const seq = useRef(0);
  const timer = useRef<number | null>(null);

  const load = () => {
    setLoading(true);
    ipc<Exported>("config_export")
      .then((r) => {
        setText(r.text);
        shaRef.current = r.sha;
        lastSent.current = r.text;
        setError(null);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };
  useEffect(load, []);

  const trySave = async (v: string) => {
    // 闸①（前端层）：解析不过就到此为止，绝不发出去。
    try {
      JSON.parse(v);
    } catch (e) {
      setError(ts("jt.parseFail", { e: String(e) }));
      return;
    }
    if (v === lastSent.current) return;
    const my = ++seq.current;
    setSaving(true);
    try {
      const r = await ipc<ImportResult>("config_import", { content: v, baseSha: shaRef.current });
      if (my !== seq.current) return;
      shaRef.current = r.sha;
      lastSent.current = v;
      setError(null);
      const kept = r.kept_protected?.length
        ? ts("jt.kept", { names: r.kept_protected.join("、") })
        : "";
      toast.ok(ts("jt.saved", { a: r.added, u: r.updated, r: r.removed, kept }));
    } catch (e) {
      if (my === seq.current) setError(String(e));
    } finally {
      if (my === seq.current) setSaving(false);
    }
  };

  const onChange = (v: string) => {
    setText(v);
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void trySave(v), 1000);
  };

  useEffect(() => () => { if (timer.current) window.clearTimeout(timer.current); }, []);

  return (
    <div className="pt-3">
      <div className="mb-2 flex min-h-6 items-center gap-3 text-sm">
        {error && (
          <span className="flex min-w-0 items-center gap-1.5 text-[var(--sem-danger-text)]">
            <IconAlert width={14} height={14} className="shrink-0" />
            <span className="break-all">{error}</span>
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-3 text-ink-4">
          {saving && <span className="animate-pulse text-ink-3">{t("save.saving")}</span>}
          {!saving && !error && !loading && <span>{t("jt.synced")}</span>}
          <button className="underline decoration-slate-600 underline-offset-2 hover:text-ink-2" onClick={load}>
            {t("jt.reload")}
          </button>
        </span>
      </div>

      <textarea
        value={text}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        className="h-[28rem] min-h-[10rem] w-full resize-y rounded-md border border-slate-800 bg-slate-950 p-3 font-mono text-xs leading-relaxed text-ink-2 focus-visible:border-cyan-600"
      />
    </div>
  );
}

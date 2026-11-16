// 一键复制（UI-AUDIT-2 E-2）：模型 id / 路径这类长串被 truncate 之后，
// 「看得见一截」不等于「读得到全文」——悬停 title 能救急，但拷不走。
// 成功/失败都要说出来，静默假装复制成功比不给复制更坏。
import { useState } from "react";
import { copyText } from "../lib/clipboard";
import { useToast } from "./Toast";
import { IconCheck, IconCopy } from "./icons";

export function CopyButton({
  text,
  label,
  className,
}: {
  text: string;
  /** 悬停文案，同时充当读屏名。 */
  label?: string;
  className?: string;
}) {
  const toast = useToast();
  const [done, setDone] = useState(false);
  const tip = label ?? `复制 ${text}`;

  const copy = async () => {
    const ok = await copyText(text);
    if (ok) {
      setDone(true);
      window.setTimeout(() => setDone(false), 1200);
      toast.ok(`已复制：${text}`);
    } else {
      toast.err("复制失败，请手动选中复制");
    }
  };

  return (
    <button
      type="button"
      title={tip}
      aria-label={tip}
      onClick={copy}
      className={`grid h-5 w-5 shrink-0 place-items-center rounded text-ink-4 transition-[opacity,background-color,color] hover:bg-slate-700/60 hover:text-ink-1${
        className ? ` ${className}` : ""
      }`}
    >
      {done ? <IconCheck width={11} height={11} /> : <IconCopy width={11} height={11} />}
    </button>
  );
}

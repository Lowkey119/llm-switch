// 「最近发送」勾选态 + 批量删除（OK 短按勾选，删除条提交）。
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { ipc } from "../lib/ipc";

export function useRecentPicks() {
  const [picked, setPicked] = useState<string[]>([]);
  const remove = useMutation({
    mutationFn: (texts: string[]) => ipc<string[]>("prompt_recent_remove", { texts }),
    onSuccess: () => setPicked([]),
  });
  const toggle = (content: string) =>
    setPicked((p) => (p.includes(content) ? p.filter((x) => x !== content) : [...p, content]));
  const clear = () => setPicked([]);
  return { picked, toggle, clear, remove };
}

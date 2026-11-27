// 快速面板的两块小状态（从 PromptPanel 拆出——该文件过了 ui-budget 的 550 行线，
// 法条是「超预算先拆，不往里塞」）。两块都不碰面板的其它状态，天然可切。
import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../lib/ipc";
import { stepZoom } from "../lib/zoom";

/**
 * 面板态特殊宏的两个事件（计划 2026-11-26 §三.2）：音量 ± 走缩放三档循环，
 * Home 键在可注入列表里循环切目标。这两件事的状态都在面板自己这边
 * （倍率存 webview localStorage、目标在前端组合框），Rust 侧够不着，
 * 所以后端只发事件、面板自己动。
 */
export function usePromptSpecials(o: {
  /** 当前目标名（Target 不暴露 pid，循环按名对齐）。 */
  targetName?: string;
  refetchTarget: () => void;
}): void {
  const { targetName, refetchTarget } = o;
  useEffect(() => {
    const cycleTarget = async () => {
      try {
        const apps = await ipc<{ pid: number; name: string }[]>("prompt_apps");
        if (!apps.length) return;
        // 按名对齐当前项再往后走一格（同名多项取第一个，循环到手感一致）。
        const i = apps.findIndex((a) => a.name === targetName);
        const next = apps[(i + 1) % apps.length];
        await ipc("prompt_target_set", { pid: next.pid });
        refetchTarget();
      } catch {
        /* 列表拿不到就算了：事件回调里抛出去没人接，只会变成未处理 promise */
      }
    };
    const unZoom = listen<number>("prompt://zoom-step", (e) => stepZoom("prompt", Number(e.payload)));
    const unCycle = listen("prompt://cycle-target", () => void cycleTarget());
    return () => {
      void unZoom.then((f) => f());
      void unCycle.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}

/**
 * 「最近发送」历史（计划 §三.7）：读、记、清空都走后端 settings（封顶 20 条）。
 * 后端三条命令都直接回最新列表，所以本地缓存就地更新，不必再 refetch。
 */
export function useRecent() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["prompt-recent"],
    queryFn: () => ipc<string[]>("prompt_recent_get"),
    staleTime: Infinity,
  });
  const put = (v: string[]) => qc.setQueryData(["prompt-recent"], v);
  return {
    /** 历史列表（新的在前）。 */
    items: q.data ?? [],
    /** 记一条发送（空白不记，后端会滤）。 */
    add: (text: string) =>
      void ipc<string[]>("prompt_recent_add", { text })
        .then(put)
        .catch(() => {}),
    /** 清空（「最近发送」tab 右键菜单入口）。 */
    clear: () =>
      void ipc<string[]>("prompt_recent_clear")
        .then(put)
        .catch(() => {}),
  };
}

/**
 * 语音编辑态（计划 §语音编辑态）：语音键按下（state=listening）就进编辑态，
 * partial / 定稿逐字写进回显框。后端同期不再 inject_text（面板可见就跳过，见 voice.rs），
 * 否则同一段话会被打两遍——这就是计划点名的「双重注入必须解决」。
 */
export function useVoiceEdit() {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  useEffect(() => {
    const un = listen<{ state?: string; partial?: string; text?: string }>("remote://voice", (e) => {
      const p = e.payload ?? {};
      if (p.state === "listening") {
        setEditing(true);
        return;
      }
      // partial 是「当前整句」不是增量，直接覆盖；定稿（injected 带 text）同样整段覆盖。
      if (typeof p.partial === "string" && p.partial) setText(p.partial);
      else if (p.state === "injected" && typeof p.text === "string" && p.text) setText(p.text);
    });
    return () => void un.then((f) => f());
  }, []);
  // 长按返回键「清空编辑框」（计划：Esc 留着内容，总得有个清空入口）。
  useEffect(() => {
    const un = listen("prompt://clear-voice", () => setText(""));
    return () => void un.then((f) => f());
  }, []);
  return {
    editing,
    text,
    setText,
    /** 退出编辑态但留着内容（Esc 口径：下次语音接着追加）。 */
    exit: () => setEditing(false),
    /** 发送后收尾：退编辑态并清空——内容已送到目标那边，留着下次会说出重复话。 */
    consume: () => {
      setEditing(false);
      setText("");
    },
  };
}

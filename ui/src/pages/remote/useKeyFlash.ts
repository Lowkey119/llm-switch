// 实体键实时联动（从 Mapping 拆出，队列 22 预算拆文件）：remote://key 事件 → 三态闪烁。
// 设置态按真键 = 选中该键并短促加亮（live）；运行态按住亮（held）、松手余辉 350ms（after）
// ——单击闪一下、连按闪两下、长按全程持续亮（down/up 天然覆盖，gesture 归 S-C 执行用）。
import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { ipcEvent } from "../../lib/ipc";

/** remote://key 事件（S-B payload）：phase = down/up/gesture，slot = 手势归类的槽名。 */
export interface KeyEvent {
  id: string;
  slot: "click" | "double" | "long" | null;
  phase: "down" | "up" | "gesture";
}

export function useKeyFlash({
  editRef,
  captureRef,
  slotRefs,
  selectKey,
}: {
  editRef: MutableRefObject<boolean>;
  captureRef: MutableRefObject<string | null>;
  slotRefs: MutableRefObject<Map<string, HTMLButtonElement>>;
  selectKey: (id: string) => void;
}) {
  const [live, setLive] = useState<string | null>(null);
  const [held, setHeld] = useState<string | null>(null);
  const [after, setAfter] = useState<string | null>(null);
  const liveTimer = useRef(0);
  const afterTimer = useRef(0);
  useEffect(() => {
    let dead = false;
    const handle = (ev: KeyEvent) => {
      if (dead || ev.phase === "gesture") return;
      const id = ev.id;
      if (editRef.current) {
        if (ev.phase !== "down") return;
        selectKey(id);
        setLive(id);
        window.clearTimeout(liveTimer.current);
        liveTimer.current = window.setTimeout(() => setLive(null), 700);
        // 设置态按真键 → 焦点进该键「单击」槽（用户 2026-11-20 要求）；
        // 正在别的槽里录入/录制时不抢焦点（看聚焦槽记账，同一口径）。
        if (!captureRef.current) {
          slotRefs.current.get(`${id}:click`)?.focus();
        }
      } else if (ev.phase === "down") {
        setHeld(id);
      } else {
        setHeld((h) => (h === id ? null : h));
        setAfter(id);
        window.clearTimeout(afterTimer.current);
        afterTimer.current = window.setTimeout(() => setAfter(null), 350);
      }
    };
    // 运行中的旧后端（S-B 之前的实例）是旧事件形状：remote://key 的 payload 是字符串键 id、
    // 松手走独立的 remote://key-up 事件。两种形状都归一成 {id, slot, phase}
    // （开发前端配旧二进制的实况兜底；新后端不发 key-up，挂了不冲突）。
    const norm = (ev: KeyEvent | string): KeyEvent =>
      typeof ev === "string" ? { id: ev, slot: null, phase: "down" } : ev;
    let un1: (() => void) | undefined;
    let un2: (() => void) | undefined;
    void ipcEvent<KeyEvent | string>("remote://key", (ev) => handle(norm(ev))).then((f) => (dead ? f() : (un1 = f)));
    void ipcEvent<string>("remote://key-up", (id) => handle({ id, slot: null, phase: "up" })).then((f) =>
      dead ? f() : (un2 = f),
    );
    return () => {
      dead = true;
      window.clearTimeout(liveTimer.current);
      window.clearTimeout(afterTimer.current);
      un1?.();
      un2?.();
    };
    // selectKey/refs 每次渲染换新引用但语义稳定（ref 记账 + 首帧闭包即可），只挂一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return { live, held, after };
}

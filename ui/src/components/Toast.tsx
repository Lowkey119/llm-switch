import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { IconAlert, IconCheck, IconClose, IconInfo } from "./icons";

/**
 * 全站唯一提示出口（UI-AUDIT P0-4）。
 *
 * 治的是四件事：以前 4 套 toast 两种位置、错误和成功同为青色（失败被伪装成成功）、
 * 裸 setTimeout 不清理导致连点两次时旧定时器提前抹掉新消息、读屏完全听不到提示。
 * 所以这里把「位置 / 类型色 / 定时器 / aria-live」四样一次定死，页面只负责说事。
 */

export type ToastKind = "success" | "error" | "info";

export interface ToastApi {
  push: (msg: string, kind?: ToastKind, ms?: number) => void;
  ok: (msg: string) => void;
  err: (msg: string) => void;
  info: (msg: string) => void;
}

const Ctx = createContext<ToastApi | null>(null);

/** 失败比成功留得久：成功只是「知道了」，失败得让人来得及读完原因。 */
const DURATION: Record<ToastKind, number> = { success: 2600, info: 2600, error: 5200 };

/** 同屏最多四条，超了丢最旧的，避免连环失败刷满屏。 */
const MAX = 4;

export function useToast(): ToastApi {
  const v = useContext(Ctx);
  if (!v) throw new Error("useToast 必须在 <ToastProvider> 内调用");
  return v;
}

interface Item {
  id: number;
  msg: string;
  kind: ToastKind;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Item[]>([]);
  const timers = useRef(new Map<number, number>());
  const seq = useRef(0);

  const dismiss = useCallback((id: number) => {
    const t = timers.current.get(id);
    if (t !== undefined) {
      window.clearTimeout(t);
      timers.current.delete(id);
    }
    setItems((xs) => xs.filter((x) => x.id !== id));
  }, []);

  const push = useCallback(
    (msg: string, kind: ToastKind = "info", ms?: number) => {
      if (!msg) return;
      const id = ++seq.current;
      setItems((xs) => [...xs, { id, msg, kind }].slice(-MAX));
      timers.current.set(
        id,
        window.setTimeout(() => dismiss(id), ms ?? DURATION[kind]),
      );
    },
    [dismiss],
  );

  // 卸载时清干净，别让定时器往已卸载的树上 setState。
  useEffect(() => {
    const map = timers.current;
    return () => {
      map.forEach((t) => window.clearTimeout(t));
      map.clear();
    };
  }, []);

  const api: ToastApi = {
    push,
    ok: (m) => push(m, "success"),
    err: (m) => push(m, "error"),
    info: (m) => push(m, "info"),
  };

  return (
    <Ctx.Provider value={api}>
      {children}
      {/* role=status + polite：提示出现时读屏会念，但不打断用户正在读的内容。 */}
      <div
        role="status"
        aria-live="polite"
        className="pointer-events-none fixed bottom-6 right-6 z-[60] flex flex-col items-end gap-2"
      >
        {items.map((t) => (
          <ToastItem key={t.id} item={t} onClose={() => dismiss(t.id)} />
        ))}
      </div>
    </Ctx.Provider>
  );
}

const TONE: Record<ToastKind, { box: string; Icon: typeof IconCheck }> = {
  // 底色统一用卡片底，只让描边与文字带语义色：实心彩底上的白字两套主题都不达标。
  success: { box: "border-emerald-500/45 text-[var(--sem-ok-text)]", Icon: IconCheck },
  error: { box: "border-rose-500/50 text-[var(--sem-danger-text)]", Icon: IconAlert },
  info: { box: "border-slate-700 text-ink-2", Icon: IconInfo },
};

function ToastItem({ item, onClose }: { item: Item; onClose: () => void }) {
  const { box, Icon } = TONE[item.kind];
  return (
    <div
      className={`pointer-events-auto flex max-w-[420px] items-start gap-2 rounded-lg border bg-slate-900 px-3.5 py-2 text-xs shadow-lg ${box}`}
    >
      <Icon width={14} height={14} className="mt-px shrink-0" />
      <span className="min-w-0 break-words">{item.msg}</span>
      <button
        onClick={onClose}
        aria-label="关闭提示"
        className="-mr-1 shrink-0 rounded p-0.5 text-ink-4 transition-colors hover:text-ink-2"
      >
        <IconClose width={12} height={12} />
      </button>
    </div>
  );
}

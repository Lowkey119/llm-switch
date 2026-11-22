// 控制台左列点亮：每个后端此刻在飞的请求数（>0 点亮，=0 压暗）。
// 口径 = 网关 monitor 活跃流（console_flow），与历史用量无关。
// 自 Console.tsx 拆出（行数预算），逻辑逐字保持：轮询失败不阻断，保持上次状态。
import { useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";

export function useConsoleFlow(): { flow: Record<string, number>; ready: boolean } {
  const [flow, setFlow] = useState<Record<string, number>>({});
  // 首轮轮询是否已回：进页自动选中「在飞后端」要等它，否则 rows 先到会误选第一行。
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let active = true;
    const tick = async () => {
      try {
        const data = await ipc<Record<string, number>>("console_flow");
        if (active) setFlow(data ?? {});
      } catch {
        /* 轮询失败不阻断，保持上次状态 */
      }
      if (active) setReady(true);
    };
    tick();
    const id = setInterval(tick, 2000);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, []);
  return { flow, ready };
}

// 浮窗无操作自动隐藏：按键/指针/重新获焦都重新计时，秒数从 settings 读。
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../lib/ipc";

export function usePanelAutohide() {
  useEffect(() => {
    let dead = false;
    let timer: number | undefined;
    let unKey: (() => void) | undefined;
    const arm = (seconds: number) => {
      if (timer) window.clearTimeout(timer);
      if (!seconds) return;
      timer = window.setTimeout(() => {
        void ipc("prompt_set_visible", { visible: false }).catch(() => {});
      }, seconds * 1000);
    };
    const bump = () => {
      void ipc<number>("prompt_autohide_get")
        .then((sec) => !dead && arm(sec))
        .catch(() => {});
    };
    bump();
    window.addEventListener("keydown", bump);
    window.addEventListener("pointermove", bump);
    window.addEventListener("focus", bump);
    void listen("remote://key", bump).then((f) => {
      if (dead) f();
      else unKey = f;
    });
    return () => {
      dead = true;
      if (timer) window.clearTimeout(timer);
      window.removeEventListener("keydown", bump);
      window.removeEventListener("pointermove", bump);
      window.removeEventListener("focus", bump);
      unKey?.();
    };
  }, []);
}

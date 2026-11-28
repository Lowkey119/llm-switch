// 浮窗右侧只读遥控器图：跟随最后按下的设备切图，实体按键亮热点。
// 从 PromptPanel 拆出，保持主面板在行数预算内。
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../lib/ipc";
import { IconMouse } from "./icons";
import { t } from "../lib/i18n";
import { RemoteImage } from "../pages/remote/RemoteImage";
import { useKeyFlash } from "../pages/remote/useKeyFlash";
import { hasRemoteImage } from "../pages/remote/keys";

/** 浮窗顶部的鼠标模式角标：图标配文字，只在开启时出现。 */
export function PromptMouseBadge() {
  const [on, setOn] = useState(false);
  useEffect(() => {
    let dead = false;
    const load = (profile?: string | null) => {
      if (!profile) return void setOn(false);
      void ipc<boolean>("remote_mouse_mode_get", { profile })
        .then((v) => !dead && setOn(v))
        .catch(() => !dead && setOn(false));
    };
    void ipc<{ bound?: { profile?: string | null } }>("remote_status")
      .then((s) => load(s.bound?.profile))
      .catch(() => {});
    let off: (() => void) | undefined;
    void listen<{ profile: string }>("remote://active-profile", (e) =>
      load(e.payload?.profile ?? null),
    ).then((f) => {
      if (dead) f();
      else off = f;
    });
    void listen<{ enabled: boolean }>("remote://mouse-mode", (e) => setOn(e.payload?.enabled ?? false));
    return () => {
      dead = true;
      off?.();
    };
  }, []);
  if (!on) return null;
  return (
    <div className="flex items-center gap-1 px-3 pt-1 text-2xs text-[var(--key-hl-text)]">
      <IconMouse className="h-3.5 w-3.5" aria-hidden />
      {t("remote.mouseMode")}
    </div>
  );
}

export function PromptRemoteImage() {
  const [profile, setProfile] = useState<string | null>(null);
  const readOnly = useRef(false);
  const { live, held, after } = useKeyFlash({ editRef: readOnly, selectKey: () => {} });

  useEffect(() => {
    let dead = false;
    void ipc<{ bound?: { profile?: string | null } }>("remote_status")
      .then((s) => !dead && setProfile(s.bound?.profile ?? null))
      .catch(() => {});
    let off: (() => void) | undefined;
    void listen<{ profile: string }>("remote://active-profile", (e) =>
      setProfile(e.payload?.profile ?? null),
    ).then((f) => {
      if (dead) f();
      else off = f;
    });
    return () => {
      dead = true;
      off?.();
    };
  }, []);

  if (!hasRemoteImage(profile)) return null;
  return (
    <div className="flex h-full shrink-0 items-start justify-center overflow-hidden border-l border-slate-800">
      <RemoteImage
        profileId={profile}
        edit={false}
        sel={null}
        hover={null}
        live={live}
        held={held}
        after={after}
        onSelect={() => {}}
        onHover={() => {}}
        className="h-full"
        imgClassName="h-full w-auto !max-w-none"
      />
    </div>
  );
}

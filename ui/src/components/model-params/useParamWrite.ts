// 参数覆盖的写路径（计划 §6.1 自 ModelParamsDialog.tsx 拆出，行为不变）：
// 保存 / 删除 / 套模板三条路共用同一套「忙态 + 范围校验 + 提示 + 刷新」口径。
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useToast } from "../Toast";
import { ipc } from "../../lib/ipc";
import {
  TEMPLATES,
  applyPushReport,
  type ParamRow,
  type PushReport,
} from "../../lib/params";
import { checkRange, type Limits } from "./meta";

export function useParamWrite({
  modelRef,
  mine,
  lim,
  onSaved,
}: {
  modelRef: string;
  /** 本模型现有覆盖（键 → 行），删除时按它定位 id。 */
  mine: Map<string, ParamRow>;
  lim: Limits;
  /** 落库成功后的收尾（收起编辑态）。 */
  onSaved: () => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [rangeErr, setRangeErr] = useState<string | null>(null);

  const refresh = () => qc.invalidateQueries({ queryKey: ["params"] });

  const save = async (k: string, v: string, p: string) => {
    let parsed: unknown;
    try {
      parsed = v.trim() === "" ? "" : JSON.parse(v);
    } catch {
      parsed = v; // 裸字符串当纯文本
    }
    // 清空值（空输入或 ""）= 删除覆盖 → 回到继承链/后端默认。
    // 空串绝不落库：存进去会被网关原样下发（top_p:"" 上游 400 事故）。
    if (parsed === "" || (typeof parsed === "string" && parsed.trim() === "")) {
      const m = mine.get(k);
      setBusy(true);
      try {
        const report = m
          ? await ipc<PushReport | null>("delete_param_override", { id: m.id })
          : null;
        refresh();
        onSaved();
        if (m)
          applyPushReport(toast, report, "delete", k, () =>
            toast.ok(`${k} 覆盖已删除，恢复后端默认`),
          );
        else toast.ok(`${k} 本就未设置，保持后端默认`);
      } catch (e) {
        toast.err(`删除失败：${e}`);
      } finally {
        setBusy(false);
      }
      return;
    }
    setBusy(true);
    const bad = checkRange(k, parsed, lim);
    if (bad) {
      setRangeErr(bad);
      setBusy(false);
      return;
    }
    try {
      const report = await ipc<PushReport | null>("upsert_param_override", {
        input: { scope: "model", ref_id: modelRef, key: k, value: parsed, policy: p },
      });
      refresh();
      onSaved();
      applyPushReport(toast, report, "save", k, () =>
        toast.ok(`${k} 已保存，即刻生效`),
      );
    } catch (e) {
      toast.err(`保存失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const drop = async (id: number) => {
    setBusy(true);
    try {
      const report = await ipc<PushReport | null>("delete_param_override", { id });
      refresh();
      onSaved();
      // 提示里说键名而不是 id：用户认的是键。
      const key = [...mine.values()].find((r) => r.id === id)?.key ?? "";
      applyPushReport(toast, report, "delete", key, () => {});
    } catch (e) {
      toast.err(`删除失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const applyTemplate = async (tpl: (typeof TEMPLATES)[number]) => {
    for (const [k, v, p] of tpl.params) {
      await save(k, v, p);
    }
    toast.ok(`模板 ${tpl.name} 已套用（${tpl.params.length} 项）`);
  };
  return { busy, rangeErr, setRangeErr, save, drop, applyTemplate };
}

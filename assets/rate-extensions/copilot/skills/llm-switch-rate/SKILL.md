---
name: llm-switch-rate
description: 查询并报告当前会话的 token 速率（decode / prefill tok/s 与上下文占用）。当用户问「现在多快」「速率多少」「上下文用了多少」，或要求看 llm-switch 的实时速率时使用。数值必须原样取自脚本输出，禁止自行估算。
---

# llm-switch 速率

速率由 llm-switch 网关统一计算（引擎原生速率优先，退回按 (后端, 模型) 纠偏系数校准的
字节口径；上下文在飞期间是估算并已钳到窗口总量）。Copilot CLI 没有可自定义的状态线，
所以速率以命令形式给出。

## 步骤

1. 运行：

   ```bash
   python3 ~/.copilot/llm-switch-rate.py --app copilot
   ```

2. 把输出**原样**报告给用户。输出形如
   `模型名  ▶ 42.3 tok/s  ◐ 1200  ctx 18.2k/200k`，其中：
   - `▶` = decode 速率（带 `~` 表示来自字节校准估算，不带表示引擎原生上报）；
   - `◐` = prefill 速率；
   - `ctx` = 已用 / 窗口总量。

3. **输出为空**表示此刻取不到 llm-switch 的数（网关没起，或发现文件
   `~/.llm-switch/rate-ep.json` 缺失）。这时如实说「现在拿不到 llm-switch 的速率」，
   **不要**改用任何本地估算口径，也不要编一个数。

## 禁止

- 不要自己用时间戳、字数或 token 数推算速率——llm-switch 只有一份计算口径。
- 不要为了「看起来有数」而编造速率。

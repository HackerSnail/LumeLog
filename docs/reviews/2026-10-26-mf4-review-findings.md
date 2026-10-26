# MF4 增量1 终审结论（2026-10-26，待修复）

审查范围：`3aa8974..fb40b12`（Plan 13，已合入 `main` `7111777`）。结论：**1 Critical + 6 Important**，增量2 前必须修。四门全绿未暴露，因为夹具是「单数据组 + 单分块 + 4 样本」。

## Critical

**C1 时间轴取全文件第一个 master → 跨组错数据 / 角度轴当秒**
- `records`/`export_csv` 用 `guard.channels.iter().find(|(_, c)| c.is_master())`，把该 master 的样本下标套到**每个**选中通道上；Vector/CANape 常按组分开记录流 → 第 2 组通道全 `null`（`degraded=false`）或张冠李戴。
- `is_master()` 是 `Master | VirtualMaster`，不区分时间/角度/距离；角度 master 时 `t*1000.0` 把"度"当毫秒，时间窗也在过滤度数。
- 修复：按选中通道的 `data_group_index` 找 `(same dg && c.is_time_channel())`；跨数据组选择返回 `invalid_argument`（"请选择同一数据组内的通道"）；DTO 增 `dataGroup`；`summary` 组键用 `(data_group, group)`。

## Important

**I2 降序分页跨 4 MiB 分块边界 → `rows[cursor] - base` 下溢**
`cells_for_rows` 要求升序下标，`records` 却传降序 → release 变 `null`、debug panic。修复：先按升序取单元格再按显示顺序回排（`slot_of` 映射），并给 `select_rows` 的降序页加 `last_needed = total - start` 扫描终点。

**I3 「按页有界」被打破**
- 无时间轴分支 `file.signal(first)` 整信号物化；
- 时间轴分支把整个时间窗 `(index, ms)` 全量 `push`（16 B/样本），且**每翻一页重扫**。
修复：`select_rows` 两遍流式——第一遍只计数、第二遍用 `HashMap<position, slot>` 只收集本页；无时间轴用 `signal_chunks` 累加长度。

**I4 导出全程持 session+file 两把锁；UI `busy` 未拦住取数入口**
导出可在 worker 内先取 `Arc<Mutex<Mf4File>>` 快照并 `drop(guard)`；`mf4-ui.ts` 的所有 `query()` 入口统一 `if (this.busy) return`。

**I5 `#mf4-dock` 缺高度契约**
`.mf4-view{height:100%}` 在 auto 高父级解析为 auto → 表格永不滚动、500 行页被裁。修复：`#mf4-dock{flex:0 0 auto;max-height:55%;min-height:0;display:flex;flex-direction:column;...}`，`.mf4-view` 改 `flex:1 1 auto;min-height:0`（无 `height:100%`）。**（已修，提交见 CHANGELOG）**

**I6 通道 >2000 静默截断**
`main.ts` 只取 `limit:2000` 且丢弃 `total`。最小修复：界面显示"共 N 个通道，仅加载前 2000，请用搜索缩小范围"；正确修复：搜索走后端分页（后端已支持）。

**I7 打开后 `channels`/`summary` 失败 → 会话泄漏；无防重入**
`mf4Open` 成功后失败路径没有 `mf4Close`，累计 4 次后所有打开被拒；连续两次打开互相覆盖 `mf4View`。修复：`opened` 变量 + `catch` 内关闭 + in-flight 标记。

## Minor（可延后）
1. `Bytes/VarBytes/Complex` 永远 `null` 但不置 `degraded`。
2. 导出 UI 无取消按钮（`mf4_export_cancel` 已就绪）。
3. `group` 只用 `channel_group_index`，多数据组时 `summary` 合并同号组（随 C1 一起改）。
4. 无时间轴：表格 `totalRows` 取第一个通道长度、导出取选中首个通道长度，可能不一致；CSV 表头仍是 `timeMs`。
5. `renderChannels()` 每次搜索都把状态区改写成「已选 N/32」，覆盖"读取中/导出 N 行"。
6. 缺测试：跨分块降序、双数据组、无时间轴、`Bytes→degraded`、`busy` 时复选框不发查询。

## 待手工验证（真实样本）
多组/角度 master 文件；>2000 通道；500/1000 行页滚动；≥1M 样本翻页与内存峰值；导出中点复选框是否冻结；导出目标被占用/只读/同源；会话上限连击；CAN/MF4 dock 互斥；`cargo +1.89 build` 与三平台 CI。

## 本轮修复进度
- 已提交：C1/I2/I3/I4（`961d28e`）、I5+审查记录（`0548b7a`）、I6/I7 与 Minor 1/2/3/4/5（`9e065c1`，含 `busy` 与取消按钮 jsdom 测、VLSD bytes `degraded` 测）。
- 仍未做（需真实样本或大夹具）：跨 4 MiB 分块降序回归测、无时间轴夹具、>2000 通道改走后端分页搜索。
- 待手工验：多组/角度 master 文件、大文件翻页、导出取消与目标占用、`cargo +1.89 build`、三平台 CI。

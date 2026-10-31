# 工作区会话恢复（Plan 40）

N++ 重启恢复上次打开的全部标签页。后端 `doc_list` 是运行态查询，
跨重启的会话归前端存：`localStorage["lumelog.session.v1"]` =
`{ paths: string[], active: string | null }`（≤50 个，去重保序）。

## 1. 写入时机

`tabs.list()`（开序）+ `tabs.active()` 快照整表落盘，挂三处：
打开成功、标签关闭完成、激活切换。每次开/关/切都写，崩溃也不丢。
无未保存缓冲概念（LumeLog 无 untitled 文档），路径即会话；
未保存改动由崩溃恢复影子另行负责。

## 2. 启动恢复

启动序列末尾 `restoreSession()`：逐个 `openPath(path, { quiet: true })`
（缺失/损坏文件静默跳过，不打错误弹窗），完成后若全部/部分成功
toast「已恢复 N 个文档」，有缺失追加「M 个文件已缺失」；
最后激活记录的 active（按路径映射回 docId）。

## 3. 验收

- `session.test.ts`：normalize（去重保序/过滤空串/上限 50/active 不在集合
  置 null）、parse 坏数据、往返；
- 前端 tsc/npm test/build 绿；
- 人工：开几个文档→重启→标签恢复且激活页正确（`tauri dev`）。

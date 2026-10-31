# 替换预览（Plan 46）

N++/VS Code 的"替换前先看命中"。编辑器（edit 模式）路径：palette
「查找并替换（预览）…」弹面板——查找/替换串、字面/正则、大小写、
范围（选区或整篇）→ 预览列表（行号 + 行摘要）与计数 → 全部替换
（单次 dispatch，一步撤销）或取消。校验失败 toast，文本不动。

## 1. 纯函数 `replacetools.ts`

- `buildReplaceRe(find, mode, case)`：literal → 元字符转义；regex → 原样
  加 g（非法 pattern 报错）；空查找报错；零长匹配跳过（防死循环）；
- `findMatches(text, re, range)`：`{from, to, line(1-based), preview(行摘要
  截 200 字符)}`，行号随扫描增量计数；
- `applyReplace(text, re, repl, literal)`：literal 用函数替换（`$` 不具义），
  regex 保留 `$1` 组引用（String.replace 语义）；
- `replacePreview(...)` 组合：`{ok, matches, result}` / `{ok:false, error}`。

## 2. 前端

- palette「查找并替换（预览）…」；复用 commit-box 面板样式；预览列表
  渲染上限 1000 行（计数仍为全量）；只读视图提示不支持。

## 3. 已知限制

- 只读大文件路径无替换（定位如此）；多行替换串仅 literal 模式推荐
  （regex 模式 `\n` 需自写）。

## 4. 验收

- `replacetools.test.ts`：转义/大小写、组引用、literal `$` 不具义、
  零长守卫、行号、范围截取、坏 pattern；
- tsc/npm test/build 绿；人工：预览→替换→撤销一步还原（`tauri dev`）。

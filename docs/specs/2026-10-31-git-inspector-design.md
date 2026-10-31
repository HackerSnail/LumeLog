# 检查器：Git 小节（仓库探测 / 初始化 / 文件状态）（Plan 34）

对齐 NoteLume 检查器的 Git 块（截图：未检测到 Git 仓库 → 初始化仓库按钮）。
只读展示 + 初始化，不做提交/暂存。走系统 git CLI（零新依赖，符合轻量定位）。

## 1. Rust：`gitinfo.rs`

- `run_git(dir, args)`：`std::process::Command`，参数数组不过 shell，
  cwd 为文档所在目录；输出非 0 视为失败；
- `git_probe(dir, file)`：
  - `rev-parse --show-toplevel` → 有仓库则得 root；
  - `symbolic-ref --short -q HEAD` → 分支名（空输出 = 分离 HEAD）；
  - `status --porcelain -- <file>` → 纯函数 `porcelain_file_status`
    解析首个 XY 到中文（??=未跟踪、首列 M/A/D/R=已暂存、次列 M/D=有未暂存修改），
    无输出 = 未改动；
  - `log -1 --format=%h%x09%s%x09%ct -- <file>` → 最近一次提交（文件级），
    空仓库/无提交 → None；
- 命令：`git_probe(path)`、`git_init(path)`（`git init` 于文档目录）。

## 2. 前端：检查器「Git」小节

- `updateInspector` 在标签小节后追加 Git 小节壳，异步探测回填
  （token 防旧结果覆盖新文档）；行：仓库根、分支、文件状态、最近提交；
- 无仓库：提示 + 「初始化仓库」按钮 → ask 确认 → `gitInit(目录)` →
  重探 + toast；
- 刷新点沿用 updateInspector 的既有调用（激活/保存/重载/编码切换）。

## 3. 限制

- 依赖 PATH 里的 git CLI，无则小节提示「git 不可用」；
- 探测无超时（index.lock 竞争时该次探测等待），on-demand 触发、
  不进任何热路径；
- 只读：不做 add/commit/push；文件状态为仓库级单文件视角。

## 4. 验收

- Rust 单测：porcelain 解析各分支；有 git CLI 时 tempdir 集成
  （无仓库探测 → init → 重探出 root/分支/未跟踪），无 CLI 跳过；
- `cargo test --workspace` + `clippy -D warnings` 净；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；
- 人工：真实仓库下激活/保存后小节刷新（`tauri dev`）。

# Git 内置化 + 暂存并提交（Plan 35）

Plan 34 只读探测走的是系统 git CLI；本 plan 两件事：
① 换 `git2`（libgit2 绑定，编译期内置，不依赖 PATH）；② 增加
「暂存并提交当前文件」。

## 1. gitinfo.rs 重写为 git2

- `Repository::discover` 探仓库；分支：`head()` 正常取 shorthand，
  空仓库（unborn HEAD）经 `find_reference("HEAD")` 的符号目标抠出
  `refs/heads/<名>`；分离 HEAD 显示 `（分离 HEAD）`；
- 文件状态：`status_file(相对路径)` 标志位 → 纯函数 `status_label`
  （暂存列优先，叠加未暂存改动时后缀提示；空 = None = 未改动）；
- 最近提交：revwalk 从 HEAD 走，逐提交比对该项在父树中的 oid，
  第一处变化即文件级最近提交；空仓库 → None；
- `identity`：`repo.config()` 的 user.name/user.email 都在才带出，
  供前端预填提交者；
- `git_stage_and_commit`：index `add_path(文件)` → `write_tree` →
  HEAD 存在则以之为父提交，`commit(Some("HEAD"), …)` 兼容空仓库；
  签名取 参数 → 仓库/全局配置，都缺报 `git_identity` 错误
  （提示配置方法）。提交的是整个暂存区（与命令行语义一致）。

## 2. 前端

- Git 小节有改动（file_status 非空）时给「暂存并提交…」按钮；
- 提交对话框（复用 palette 遮罩样式）：提交信息 textarea（必填）、
  姓名/邮箱（预填 `identity`，留空走后端配置回退）；提交成功 toast
  短 hash 并刷新小节。

## 3. 限制

- libgit2 不跑仓库钩子、不支持 GPG 签名提交；
- 提交包含暂存区全部内容，不只当前文件（与 git 命令行一致）；
- 体积：git2/libgit2-sys 进二进制（交付注记记录 release 体积账）。

## 4. 验收

- Rust 单测：status_label 各分支；tempdir 集成 init → 提交（显式身份）→
  探测转干净 + last_commit 命中新提交；身份缺省路径按配置注入校验；
- `cargo test --workspace` + clippy -D warnings 净；
- 前端 tsc/npm test/build 绿；
- 人工：真实仓库提交后小节与外部 git 命令核对（`tauri dev`）。

# Repository Guidelines

## 项目结构与模块组织

- `src/`：Vite 前端（原生 JS + CSS）。入口 `src/main.js`，样式 `src/style.css`。
- `src-tauri/`：Tauri（v2）Rust 后端。
  - `src-tauri/src/main.rs`：应用入口 + 全部 `#[tauri::command]` IPC 接口。
  - `src-tauri/src/models.rs`：数据模型（配置、账号、结果等）。
  - `src-tauri/src/state.rs`：本地持久化（`balance_cache.json`、`daily_web_login_state.json`）。
  - `src-tauri/src/monitor.rs` / `src-tauri/src/web_native.rs`：批量查询编排 + 原生浏览器自动化。
- `dist-release/`：打包产物目录（不作为开发源代码）。

## 构建、测试与本地开发命令

```bash
# 安装前端依赖（所有 dev/build 流程都需要）
npm install

# 开发模式（Vite dev server + Tauri 后端）
npm run tauri:dev

# 生产构建（Tauri 打包）
npm run tauri:build

# 仅启动前端（不带 Tauri 后端）
npm run dev

# 后端检查（从仓库根目录执行）
cd src-tauri && cargo test
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy
```

- Windows 便捷脚本：`start_art_rs.bat`（dev）或 `start_art_rs.bat build`（build）。

## 常见问题（排障）

- 现象：`'tauri' is not recognized as an internal or external command`  
  处理：通常是 devDependencies 未安装（`@tauri-apps/cli` 缺失）。在仓库根目录执行：`npm install --include=dev`（或 `npm install --production=false`），并确认 `node_modules/.bin/tauri.cmd` 存在。

## 编码风格与命名约定

- Rust：edition 2024；用 rustfmt 格式化（`cargo fmt`）；错误处理优先 `anyhow::Result` + `.with_context()`；日志使用 `tracing`。
- 约定：UI 文案、日志、错误信息以中文为主；避免在代码/日志中使用 emoji（项目惯例）。
- JS：保持现有原生写法（2 空格缩进、ES modules、分号）；UI 以模板字符串渲染为主，DOM id/class 命名需清晰可检索。

## 测试指南

- 当前未配置专门的前端测试框架；修改 UI 后通过 `npm run tauri:dev` 手动走通相关流程做验证。
- 后端测试使用 Rust 内置测试（`cargo test`）。建议在逻辑密集模块旁增加 `#[cfg(test)] mod tests { ... }` 单元测试（如配置解析、切日规则、状态持久化）。

## Commit 与 Pull Request 指南

- 当前 Git 历史较少，commit subject 多为短摘要（如“`一些优化`”）；请保持主题简短且可读，必要时可加模块前缀（例：`tauri: 修复配置加载`）。
- PR 建议包含：变更内容/原因、可复现的验证步骤（命令 + UI 操作）、UI 改动截图；如涉及运行时配置或环境变量变更务必说明。

## 配置与本地数据

- 运行时文件（通常位于配置目录）：`config.json`、`credentials.txt`、`balance_cache.json`、`daily_web_login_state.json`。
- 仓库提供示例：`config.example.json`、`credentials.example.txt`；首次使用可复制为实际文件名后再运行。
- 可用 `ART_RS_CONFIG_DIR` 指定本地配置目录；请勿提交真实账号/密钥到仓库。
- Dev server 端口可通过 `VITE_DEV_SERVER_PORT` 覆盖。

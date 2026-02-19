# ART-rs（Tauri 重构版）

ART-rs 是一个基于 **Tauri v2 + Rust** 的桌面应用，用于批量登录/查询 AnyRouter 账号余额，并提供缓存、自动轮询、结果汇总与 Key 管理能力（面向日常使用场景）。

## 主要功能
- 余额查询：支持查询全部账号或指定账号；API 秒查失败可回退到网页/缓存（受配置控制）。
- 自动轮询：按间隔定时刷新，底部状态栏显示倒计时。
- 结果操作：复制 API Key、设置 Claude Token（写入 `~/.claude/settings.json`）、设置 OpenAI Key（写入 `~/.codex/auth.json`，并尝试同步 WSL）。
- Claude 低余额自动换 Key：当 **当前 Claude Token** 余额低于阈值时，自动切换到余额最高的 Key（仅影响 Claude）。
- 数据展示增强（余额查询页）：搜索/筛选/排序、汇总标签（数量/小计/平均，支持紧凑模式；`Shift + 点击` 可复制该状态账号列表）、复制 CSV/JSON/失败账号（位于“更多”菜单）、键盘快捷键（`/` 聚焦搜索，`Esc` 关闭菜单/清空）。

## 目录结构
- `src/`：Vite 前端（原生 JS + CSS），入口 `src/main.js`、样式 `src/style.css`。
- `src-tauri/`：Tauri Rust 后端（Tauri commands + 业务逻辑）。
- `start_art_rs.bat`：Windows 一键启动脚本（自动端口、自动补齐 devDependencies、前后清理）。
- `config.example.json` / `credentials.example.txt`：示例配置（用于 GitHub）。

## 本地开发与运行

### 1) 首次准备（推荐）
```bash
# 安装依赖（需要 devDependencies 才能运行 tauri dev）
npm install --include=dev

# 复制示例文件（不要提交真实账号/密钥）
# Git Bash / macOS / Linux：
cp credentials.example.txt credentials.txt
cp config.example.json config.json # 可选
#
# Windows PowerShell：
# Copy-Item credentials.example.txt credentials.txt
# Copy-Item config.example.json config.json
```

### 2) 运行（Windows 一键）
```bat
start_art_rs.bat
```
可选 `build`：
```bat
start_art_rs.bat build
```

### 3) 运行（手动）
```bash
npm run tauri:dev
```
仅启动前端（不带 Tauri 后端）：
```bash
npm run dev
```

### 4) 后端检查（可选）
```bash
cd src-tauri && cargo test
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy
```

## 配置与本地数据（重要）
- 配置目录优先级：`--config-dir` > `ART_RS_CONFIG_DIR` > 当前目录 > 可执行文件目录（开发模式会兼容 `src-tauri`）。
- 运行时文件（已加入 `.gitignore`，不要提交）：
  - `config.json`（可选）
  - `credentials.txt`（必需）
  - `balance_cache.json`
  - `daily_web_login_state.json`
  - `*.log`


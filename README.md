# ART-rs（Tauri 重构版）

日期：2026-02-15  
执行者：Codex

## 项目目标
- 使用 `Tauri` 完整承载 GUI 与本地后端能力，不再依赖 Python GUI 兼容层。
- 复用 Rust 业务模块，保持原项目查询流程：
  - 账号配置加载与编辑（`credentials.txt`）
  - API 秒查余额
  - 每天首查网页登录签到（按 `daily_rollover_hour`，默认 08:00 切日）
  - 余额缓存与每日首查状态文件持久化
  - 首个 API Key 额度同步逻辑

## 目录结构
- `src-tauri/`：Tauri Rust 后端
  - `src/main.rs`：应用入口 + Tauri commands
  - `src/monitor.rs`、`src/web_native.rs` 等：核心业务逻辑
  - `tauri.conf.json`：Tauri 配置
- `src/`：前端页面（Vite + 原生 JS）
- `start_art_rs.bat`：Windows 一键启动脚本（自动端口 + 前后清理）

## 已对齐能力
- 查询与轮询：
  - 查询全部账号 / 指定账号
  - 自动轮询查询
  - 查询进度日志展示
- 账号管理：
  - 重新加载账号
  - 新增 / 更新 / 删除账号
  - 填充选中账号到编辑区
- 结果操作：
  - 复制 API Key
  - 写入 Claude Token（`~/.claude/settings.json`）
  - 写入 OpenAI Key（`~/.codex/auth.json`，并尝试同步 WSL）
- 监控能力：
  - 性能报告查看
  - 浏览器池复用与清理

## 运行方式

### 1. 一键启动（Windows）
```bat
start_art_rs.bat
```

默认模式为 `dev`，可选 `build`：
```bat
start_art_rs.bat build
```

脚本行为：
- 启动前清理锁文件与临时文件
- 自动探测空闲端口
- 动态生成 Tauri dev 配置（不硬编码端口）
- 退出后清理运行时文件

### 2. 手动启动
```bash
npm install
npm run tauri:dev
```

## 配置说明
- 默认配置目录自动探测优先级：
  1. `--config-dir <path>` 启动参数
  2. 环境变量 `ART_RS_CONFIG_DIR`
  3. 当前目录（`ART-rs`）
  4. 可执行文件目录
  5. `src-tauri`/`ART-rs` 目录（开发模式）
- 项目按“独立仓库”设计，不依赖上级目录配置文件。
- 使用文件：
  - `ART-rs/config.json`
  - `ART-rs/credentials.txt`
  - `ART-rs/balance_cache.json`
  - `ART-rs/daily_web_login_state.json`

## 技术说明
- 前端：Vite + 原生 JS + Tauri API
- 后端：Rust + Tauri commands + Tokio
- 浏览器自动化：`thirtyfour + chromedriver`
- 日志：`tracing + tracing-appender`

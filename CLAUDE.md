# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
# Install frontend dependencies
npm install

# Development mode (starts Vite dev server + Tauri backend)
npm run tauri:dev

# Production build
npm run tauri:build

# Frontend only (Vite dev server)
npm run dev

# Windows one-click launch (auto port detection, cleanup)
start_art_rs.bat           # dev mode
start_art_rs.bat build     # production build
```

Rust edition 2024, Tauri v2. No unit tests currently configured.

## Architecture

ART-rs is a Tauri desktop app for managing AnyRouter accounts: querying balances via API, performing daily web login sign-in via browser automation, caching results, and syncing API Key quotas.

### Backend (src-tauri/src/)

```
main.rs        - Tauri application entry, AppState definition, all #[tauri::command] handlers
config.rs      - RuntimeFiles paths, load/save credentials.txt and config.json
models.rs      - All data structures: AppConfig (nested: BrowserConfig, PerformanceConfig,
                 ApiConfig, LoggingConfig, WebCheckConfig), Account, CheckResult,
                 BalanceCacheRecord, BalanceCacheFile, DailyWebStateFile
state.rs       - StateStore: balance cache + daily web login state persistence,
                 cycle-day rollover logic (daily_rollover_hour)
monitor.rs     - Batch account checking orchestrator, concurrent with Semaphore,
                 supports Normal and WebOnly query modes
api_client.rs  - ApiBalanceClient: REST balance queries via billing routes,
                 fallback candidate endpoints, response header/body balance extraction
web_check.rs   - Web check dispatcher: delegates to external command (if configured)
                 or native Rust browser automation
web_native.rs  - Native browser automation via thirtyfour (Selenium WebDriver):
                 login flow, balance extraction via JS, API Key quota sync
browser_pool.rs - ChromeDriver process pool (global singleton), acquire/release tickets
driver_manager.rs - Auto-detect Chrome version, download matching ChromeDriver,
                    cache management
performance_monitor.rs - Global performance metrics collection, operation timers,
                         system resource monitoring, report generation
```

### Frontend (src/)

- `main.js` - Vanilla JS UI, calls Tauri commands via `@tauri-apps/api`
- `style.css` - Application styles
- Vite dev server on `127.0.0.1:5173` (configurable via `VITE_DEV_SERVER_PORT` env)

### Tauri Commands (IPC interface)

```
get_snapshot_command       - Returns AppSnapshot (config + accounts + cached results)
reload_accounts_command    - Re-read credentials.txt from disk
upsert_account_command     - Add or update account
remove_account_command     - Delete account
query_balances_command     - Full balance check (API + web login if needed)
web_login_only_command     - Force web login only (no API)
get_cached_results_command - Return cached balance results
save_claude_token_command  - Write API key to ~/.claude/settings.json
save_openai_key_command    - Write API key to ~/.codex/auth.json (+ WSL sync)
performance_report_command - Generate performance report string
```

### Key Data Flow

1. **Balance query**: `monitor::check_accounts` -> per-account `check_single_account`
2. **Per-account logic**: Check if daily web login needed (cycle day) -> API fast query -> web login fallback -> update StateStore cache
3. **Web login**: `web_check::run_web_check` -> either external command or `web_native::run_native_web_check` (thirtyfour + chromedriver)
4. **State persistence**: `StateStore` manages `balance_cache.json` and `daily_web_login_state.json` with atomic writes (write tmp + rename)

### Configuration Files (in config_dir)

- `config.json` - AppConfig (all defaults via serde, works with empty/missing file)
- `credentials.txt` - CSV-like: `username,password,api_key(optional)`, `#` for comments
- `balance_cache.json` - Cached balance results per account
- `daily_web_login_state.json` - Tracks which accounts completed daily web login

### Concurrency Model

- `AppState` uses `Arc<RwLock<>>` for config/accounts, `Arc<Mutex<>>` for StateStore
- `query_lock: Mutex<()>` prevents concurrent batch queries
- `monitor.rs` uses `Semaphore` to limit concurrent account checks (`max_workers`)
- `BrowserPool` is a global singleton (`OnceLock`) managing chromedriver processes

## Code Conventions

- All UI text, log messages, and error messages are in Chinese
- No emoji in code (per project convention - encoding safety)
- Rust edition 2024
- Error handling: `anyhow::Result` throughout, `.with_context()` for error context
- Logging: `tracing` crate (info/warn/debug levels)
- JSON serialization: `serde` + `serde_json` everywhere
- Async runtime: Tokio (full features)

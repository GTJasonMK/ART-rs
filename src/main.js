import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";

const MAX_PROGRESS_LOGS = 600;

const state = {
  configDir: "",
  queryInterval: 60,
  dailyRolloverHour: 8,
  fallbackToWeb: true,
  selectedUsername: "",
  accounts: [],
  results: [],
  totalBalance: 0,
  totalBalanceCount: 0,
  statusText: "\u5c31\u7eea",
  statusState: "ok",
  lastFinished: "-",
  isRunning: false,
  autoMode: false,
  autoCountdown: 0,
  autoTimer: null,
  activeTab: "query",
  progressLogs: [],
  openDropdown: null,
  editingUsername: "",
  claudeAccount: ""
};

let statusRenderPending = false;
let logsRenderPending = false;

const app = document.querySelector("#app");

app.innerHTML = `
  <div class="app">
    <header class="app-header">
      <span class="app-title">AnyRouter ART-rs</span>
      <div class="header-meta">
        <span id="metaConfigDir"></span>
        <span id="metaAccountCount"></span>
        <span id="metaClaudeAccount"></span>
        <span id="metaFinished"></span>
      </div>
    </header>

    <nav class="tab-bar">
      <button class="tab-item active" data-tab="query">\u4f59\u989d\u67e5\u8be2</button>
      <button class="tab-item" data-tab="accounts">\u8d26\u53f7\u7ba1\u7406</button>
      <button class="tab-item" data-tab="logs">\u8fd0\u884c\u65e5\u5fd7</button>
    </nav>

    <div class="tab-content">
      <!-- Tab 1: \u4f59\u989d\u67e5\u8be2 -->
      <div class="tab-pane active" id="pane-query">
        <div class="toolbar">
          <div class="toolbar-group">
            <label>\u8d26\u53f7</label>
            <select id="accountSelect"></select>
          </div>
          <button id="btnQuery" class="primary">\u67e5\u8be2</button>
          <button id="btnWebLogin">\u4ec5\u7f51\u9875\u767b\u5f55</button>
          <span class="toolbar-divider"></span>
          <div class="toolbar-group">
            <button id="btnAuto">\u5f00\u542f\u81ea\u52a8</button>
            <label>\u95f4\u9694</label>
            <input id="intervalInput" type="number" min="1" max="86400" value="60" />
            <label>\u79d2</label>
          </div>
          <span class="toolbar-divider"></span>
          <span id="totalBadge" class="total-badge" style="display:none" title="\u70b9\u51fb\u590d\u5236">\u603b\u4f59\u989d: -</span>
        </div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>\u8d26\u53f7</th>
                <th>\u72b6\u6001</th>
                <th>\u4f59\u989d</th>
                <th>\u6765\u6e90</th>
                <th>\u8bf4\u660e</th>
                <th style="width:48px"></th>
              </tr>
            </thead>
            <tbody id="resultsBody"></tbody>
          </table>
        </div>
      </div>

      <!-- Tab 2: \u8d26\u53f7\u7ba1\u7406 -->
      <div class="tab-pane" id="pane-accounts">
        <div class="account-form">
          <h3 id="formTitle">\u65b0\u589e\u8d26\u53f7</h3>
          <div class="form-row">
            <div class="form-field">
              <label>\u7528\u6237\u540d</label>
              <input id="editUsername" type="text" placeholder="\u7528\u6237\u540d" />
            </div>
            <div class="form-field">
              <label>\u5bc6\u7801</label>
              <input id="editPassword" type="password" placeholder="\u5bc6\u7801" />
            </div>
            <div class="form-field">
              <label>API Key</label>
              <input id="editApiKey" type="text" placeholder="\u53ef\u9009" />
            </div>
            <button id="btnSaveAccount" class="primary">\u4fdd\u5b58</button>
            <button id="btnCancelEdit">\u53d6\u6d88</button>
          </div>
        </div>
        <div class="toolbar">
          <button id="btnReload">\u91cd\u65b0\u52a0\u8f7d\u8d26\u53f7</button>
          <span class="toolbar-divider"></span>
          <span id="modeLabel" style="font-size:12px;color:var(--text-muted)"></span>
        </div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>\u8d26\u53f7</th>
                <th>\u5bc6\u7801</th>
                <th>API Key</th>
                <th style="width:100px">\u64cd\u4f5c</th>
              </tr>
            </thead>
            <tbody id="accountsBody"></tbody>
          </table>
        </div>
      </div>

      <!-- Tab 3: \u8fd0\u884c\u65e5\u5fd7 -->
      <div class="tab-pane" id="pane-logs">
        <div class="logs-container">
          <div class="logs-body" id="logsBody">
            <div class="logs-empty">\u6682\u65e0\u65e5\u5fd7</div>
          </div>
          <div class="logs-toolbar">
            <button id="btnClearLogs">\u6e05\u7a7a</button>
            <button id="btnPerf">\u6027\u80fd\u62a5\u544a</button>
            <span class="toolbar-divider"></span>
            <span id="logCount" class="logs-count"></span>
          </div>
        </div>
      </div>
    </div>

    <footer class="status-bar" id="statusBar" data-state="ok">
      <span id="statusText">\u5c31\u7eea</span>
      <div class="status-right">
        <span id="autoLabel"></span>
      </div>
    </footer>
  </div>
`;

const refs = {
  metaConfigDir: el("metaConfigDir"),
  metaAccountCount: el("metaAccountCount"),
  metaClaudeAccount: el("metaClaudeAccount"),
  metaFinished: el("metaFinished"),
  accountSelect: el("accountSelect"),
  intervalInput: el("intervalInput"),
  btnQuery: el("btnQuery"),
  btnWebLogin: el("btnWebLogin"),
  btnAuto: el("btnAuto"),
  totalBadge: el("totalBadge"),
  resultsBody: el("resultsBody"),
  formTitle: el("formTitle"),
  editUsername: el("editUsername"),
  editPassword: el("editPassword"),
  editApiKey: el("editApiKey"),
  btnSaveAccount: el("btnSaveAccount"),
  btnCancelEdit: el("btnCancelEdit"),
  btnReload: el("btnReload"),
  modeLabel: el("modeLabel"),
  accountsBody: el("accountsBody"),
  logsBody: el("logsBody"),
  logCount: el("logCount"),
  btnClearLogs: el("btnClearLogs"),
  btnPerf: el("btnPerf"),
  statusBar: el("statusBar"),
  statusText: el("statusText"),
  autoLabel: el("autoLabel")
};

bindEvents();
boot().catch((error) => {
  pushLog(`\u521d\u59cb\u5316\u5931\u8d25: ${toErrorMessage(error)}`);
  setStatus(`\u521d\u59cb\u5316\u5931\u8d25: ${toErrorMessage(error)}`, "error");
});

// ========== Events ==========

function bindEvents() {
  // Tab \u5207\u6362
  document.querySelector(".tab-bar").addEventListener("click", (e) => {
    const btn = e.target.closest(".tab-item");
    if (!btn) return;
    switchTab(btn.dataset.tab);
  });

  // \u5168\u5c40\u70b9\u51fb\u5173\u95ed\u4e0b\u62c9\u83dc\u5355
  document.addEventListener("click", (e) => {
    if (state.openDropdown && !e.target.closest(".cell-actions")) {
      closeDropdown();
    }
  });

  // \u8d26\u53f7\u7b5b\u9009
  refs.accountSelect.addEventListener("change", () => {
    state.selectedUsername = refs.accountSelect.value;
  });

  // \u95f4\u9694\u8c03\u6574
  refs.intervalInput.addEventListener("change", () => {
    const v = Math.max(1, Number(refs.intervalInput.value || "60"));
    state.queryInterval = v;
    refs.intervalInput.value = String(v);
    if (state.autoMode) {
      state.autoCountdown = 0;
      scheduleStatusRender();
    }
  });

  // \u67e5\u8be2\u64cd\u4f5c
  refs.btnQuery.addEventListener("click", () => runQuery());
  refs.btnWebLogin.addEventListener("click", () => runWebLoginOnly());
  refs.btnAuto.addEventListener("click", () => toggleAutoMode());

  // \u603b\u4f59\u989d\u590d\u5236
  refs.totalBadge.addEventListener("click", async () => {
    if (state.totalBalanceCount === 0) return;
    const text = `$${state.totalBalance.toFixed(2)}`;
    await navigator.clipboard.writeText(text);
    setStatus(`\u5df2\u590d\u5236\u603b\u4f59\u989d: ${text}`, "ok");
    pushLog(`\u5df2\u590d\u5236\u603b\u4f59\u989d: ${text}`);
  });

  // \u7ed3\u679c\u8868\u683c\u4e09\u70b9\u83dc\u5355
  refs.resultsBody.addEventListener("click", onResultsAction);

  // \u8d26\u53f7\u7ba1\u7406
  refs.btnReload.addEventListener("click", () => reloadAccounts());
  refs.btnSaveAccount.addEventListener("click", () => saveAccountFromEditor());
  refs.btnCancelEdit.addEventListener("click", () => cancelEdit());
  refs.accountsBody.addEventListener("click", onAccountsAction);

  // \u65e5\u5fd7
  refs.btnClearLogs.addEventListener("click", () => {
    state.progressLogs = [];
    scheduleLogsRender();
  });
  refs.btnPerf.addEventListener("click", async () => {
    try {
      const report = await invoke("performance_report_command");
      pushLog("\u6027\u80fd\u62a5\u544a\u5df2\u5237\u65b0");
      alert(report);
    } catch (error) {
      setStatus(`\u8bfb\u53d6\u6027\u80fd\u62a5\u544a\u5931\u8d25: ${toErrorMessage(error)}`, "error");
    }
  });
}

// ========== Boot ==========

async function boot() {
  setStatus("\u6b63\u5728\u52a0\u8f7d\u914d\u7f6e...", "busy");
  await setupProgressListener();
  const snapshot = await invoke("get_snapshot_command");
  hydrateFromSnapshot(snapshot);
  await refreshClaudeAccount();
  renderAll();
  pushLog("\u521d\u59cb\u5316\u5b8c\u6210");
  setStatus("\u5c31\u7eea", "ok");
}

async function setupProgressListener() {
  await listen("progress-log", (event) => {
    const { username, message } = event.payload;
    const prefix = username ? `[${username}] ` : "";
    pushLog(`${prefix}${message}`);
  });
}

async function refreshClaudeAccount() {
  try {
    state.claudeAccount = await invoke("get_current_claude_account_command");
  } catch (_) {
    state.claudeAccount = "";
  }
}

function hydrateFromSnapshot(snapshot) {
  state.configDir = snapshot.config_dir || "";
  state.queryInterval = Math.max(1, Number(snapshot.query_interval || 60));
  state.dailyRolloverHour = Number(snapshot.daily_rollover_hour || 8);
  state.fallbackToWeb = Boolean(snapshot.fallback_to_web);
  state.accounts = Array.isArray(snapshot.accounts) ? snapshot.accounts : [];
  state.results = Array.isArray(snapshot.cached_results) ? snapshot.cached_results : [];
  recalculateTotals();
  state.selectedUsername = "";
  refs.intervalInput.value = String(state.queryInterval);
}

// ========== Tab ==========

function switchTab(tabId) {
  state.activeTab = tabId;
  document.querySelectorAll(".tab-item").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.tab === tabId);
  });
  document.querySelectorAll(".tab-pane").forEach((pane) => {
    pane.classList.toggle("active", pane.id === `pane-${tabId}`);
  });
  if (tabId === "logs") {
    scheduleLogsRender();
  }
  if (tabId === "accounts") {
    renderAccountsTable();
  }
}

// ========== Render ==========

function renderAll() {
  renderMeta();
  renderAccountSelect();
  renderResults();
  renderAccountsTable();
  renderTotalBadge();
  scheduleStatusRender();
}

function renderMeta() {
  refs.metaConfigDir.textContent = state.configDir || "-";
  refs.metaAccountCount.textContent = `${state.accounts.length} \u4e2a\u8d26\u53f7`;
  refs.metaClaudeAccount.textContent = state.claudeAccount
    ? `Claude: ${state.claudeAccount}`
    : "Claude: \u672a\u914d\u7f6e";
  refs.metaFinished.textContent = `\u4e0a\u6b21: ${state.lastFinished}`;
  refs.modeLabel.textContent =
    `\u5207\u65e5: ${String(state.dailyRolloverHour).padStart(2, "0")}:00 | API\u5931\u8d25\u56de\u9000\u7f51\u9875: ${state.fallbackToWeb ? "\u542f\u7528" : "\u7981\u7528"}`;
}

function renderAccountSelect() {
  const opts = [`<option value="">\u5168\u90e8\u8d26\u53f7</option>`];
  state.accounts.forEach((item) => {
    const sel = item.username === state.selectedUsername ? "selected" : "";
    opts.push(`<option value="${esc(item.username)}" ${sel}>${esc(item.username)}</option>`);
  });
  refs.accountSelect.innerHTML = opts.join("");
}

function renderResults() {
  if (state.results.length === 0) {
    refs.resultsBody.innerHTML = `<tr><td colspan="6" class="empty-state">\u6682\u65e0\u6570\u636e\uff0c\u8bf7\u5148\u67e5\u8be2</td></tr>`;
    return;
  }
  refs.resultsBody.innerHTML = state.results.map((item) => {
    const dotClass = item.source === "cache" ? "cache"
      : item.success ? "ok"
      : item.source === "-" ? "idle"
      : "fail";
    const dotText = item.source === "cache" ? "\u7f13\u5b58"
      : item.source === "-" ? "\u5f85\u673a"
      : item.success ? "\u6210\u529f" : "\u5931\u8d25";
    return `
      <tr>
        <td>${esc(item.username)}</td>
        <td><span class="status-dot ${dotClass}">${dotText}</span></td>
        <td class="balance-value">${esc(item.balance_text || "-")}</td>
        <td>${esc(item.source || "-")}</td>
        <td>${esc(item.message || "-")}</td>
        <td class="cell-actions">
          <button class="btn-more" data-username="${escAttr(item.username)}" title="\u64cd\u4f5c">\u00b7\u00b7\u00b7</button>
          <div class="dropdown" data-menu="${escAttr(item.username)}">
            <button class="dropdown-item" data-action="copy_key" data-username="${escAttr(item.username)}">\u590d\u5236 API Key</button>
            <button class="dropdown-item" data-action="set_claude" data-username="${escAttr(item.username)}">\u8bbe\u4e3a Claude Token</button>
            <button class="dropdown-item" data-action="set_openai" data-username="${escAttr(item.username)}">\u8bbe\u4e3a OpenAI Key</button>
            <div class="dropdown-sep"></div>
            <button class="dropdown-item danger" data-action="delete_account" data-username="${escAttr(item.username)}">\u5220\u9664\u8d26\u53f7</button>
          </div>
        </td>
      </tr>
    `;
  }).join("");
}

function renderTotalBadge() {
  if (state.totalBalanceCount > 0) {
    refs.totalBadge.style.display = "";
    refs.totalBadge.textContent = `\u603b\u4f59\u989d: $${state.totalBalance.toFixed(2)} (${state.totalBalanceCount}\u4e2a)`;
  } else {
    refs.totalBadge.style.display = "none";
  }
}

function renderAccountsTable() {
  if (state.accounts.length === 0) {
    refs.accountsBody.innerHTML = `<tr><td colspan="4" class="empty-state">\u6682\u65e0\u8d26\u53f7</td></tr>`;
    return;
  }
  refs.accountsBody.innerHTML = state.accounts.map((item) => `
    <tr>
      <td>${esc(item.username)}</td>
      <td class="td-masked">${maskText(item.password)}</td>
      <td class="td-masked">${item.api_key ? maskText(item.api_key) : "-"}</td>
      <td>
        <button class="ghost" data-action="edit" data-username="${escAttr(item.username)}">\u7f16\u8f91</button>
        <button class="danger" data-action="delete" data-username="${escAttr(item.username)}">\u5220\u9664</button>
      </td>
    </tr>
  `).join("");
}

// ========== Dropdown ==========

function onResultsAction(e) {
  // \u4e09\u70b9\u6309\u94ae
  const moreBtn = e.target.closest(".btn-more");
  if (moreBtn) {
    e.stopPropagation();
    const username = moreBtn.dataset.username;
    const menu = moreBtn.nextElementSibling;
    if (state.openDropdown === menu) {
      closeDropdown();
    } else {
      closeDropdown();
      menu.classList.add("open");
      state.openDropdown = menu;
    }
    return;
  }

  // \u83dc\u5355\u9879\u70b9\u51fb
  const menuItem = e.target.closest(".dropdown-item");
  if (menuItem) {
    e.stopPropagation();
    closeDropdown();
    const action = menuItem.dataset.action;
    const username = menuItem.dataset.username;
    if (!username) return;
    handleResultAction(action, username);
  }
}

function closeDropdown() {
  if (state.openDropdown) {
    state.openDropdown.classList.remove("open");
    state.openDropdown = null;
  }
}

async function handleResultAction(action, username) {
  if (action === "copy_key") await copyApiKey(username);
  else if (action === "set_claude") await setClaudeToken(username);
  else if (action === "set_openai") await setOpenAiToken(username);
  else if (action === "delete_account") await deleteAccount(username);
}

// ========== Accounts Tab ==========

function onAccountsAction(e) {
  const btn = e.target.closest("button[data-action]");
  if (!btn) return;
  const action = btn.dataset.action;
  const username = btn.dataset.username;
  if (!username) return;
  if (action === "edit") fillEditor(username);
  else if (action === "delete") deleteAccount(username);
}

function fillEditor(username) {
  const account = state.accounts.find((item) => item.username === username);
  if (!account) return;
  state.editingUsername = username;
  refs.formTitle.textContent = `\u7f16\u8f91\u8d26\u53f7: ${username}`;
  refs.editUsername.value = account.username || "";
  refs.editPassword.value = account.password || "";
  refs.editApiKey.value = account.api_key || "";
}

function cancelEdit() {
  state.editingUsername = "";
  refs.formTitle.textContent = "\u65b0\u589e\u8d26\u53f7";
  refs.editUsername.value = "";
  refs.editPassword.value = "";
  refs.editApiKey.value = "";
}

async function saveAccountFromEditor() {
  const username = refs.editUsername.value.trim();
  const password = refs.editPassword.value.trim();
  const apiKey = refs.editApiKey.value.trim();
  if (!username || !password) {
    setStatus("\u7528\u6237\u540d\u548c\u5bc6\u7801\u4e0d\u80fd\u4e3a\u7a7a", "warn");
    return;
  }
  try {
    const response = await invoke("upsert_account_command", {
      username,
      password,
      apiKey: apiKey || null,
      api_key: apiKey || null
    });
    state.accounts = response.accounts || [];
    cancelEdit();
    renderMeta();
    renderAccountSelect();
    renderAccountsTable();
    setStatus(response.message || "\u4fdd\u5b58\u5b8c\u6210", "ok");
    pushLog(response.message || "\u4fdd\u5b58\u5b8c\u6210");
  } catch (error) {
    setStatus(`\u4fdd\u5b58\u8d26\u53f7\u5931\u8d25: ${toErrorMessage(error)}`, "error");
  }
}

// ========== Query ==========

async function runQuery() {
  if (state.isRunning) return;
  state.isRunning = true;
  scheduleStatusRender();
  const target = state.selectedUsername || null;
  const title = target ? `\u67e5\u8be2\u8d26\u53f7: ${target}` : `\u67e5\u8be2\u5168\u90e8 ${state.accounts.length} \u4e2a\u8d26\u53f7`;
  pushLog("==================================================");
  pushLog(title);
  setStatus("\u67e5\u8be2\u4e2d...", "busy");
  try {
    const r = await invoke("query_balances_command", {
      targetUsername: target,
      target_username: target
    });
    state.results = r.results || [];
    recalculateTotals();
    state.lastFinished = r.finished_at || "-";
    renderMeta();
    renderResults();
    renderTotalBadge();
    pushLog(`\u5b8c\u6210: \u6210\u529f ${r.success_count} / \u5931\u8d25 ${r.fail_count}`);
    if (r.total_balance_count > 0) {
      pushLog(`\u603b\u4f59\u989d: $${Number(r.total_balance || 0).toFixed(2)}`);
    }
    pushLog("==================================================");
    setStatus(`\u67e5\u8be2\u5b8c\u6210\uff0c\u8017\u65f6 ${Number(r.elapsed_secs || 0).toFixed(2)}s`, "ok");
  } catch (error) {
    setStatus(`\u67e5\u8be2\u5931\u8d25: ${toErrorMessage(error)}`, "error");
    pushLog(`\u67e5\u8be2\u5931\u8d25: ${toErrorMessage(error)}`);
  } finally {
    state.isRunning = false;
    if (state.autoMode) state.autoCountdown = state.queryInterval;
    scheduleStatusRender();
  }
}

async function runWebLoginOnly() {
  if (state.isRunning) return;
  state.isRunning = true;
  scheduleStatusRender();
  const target = state.selectedUsername || null;
  const title = target
    ? `\u4ec5\u7f51\u9875\u767b\u5f55: ${target}`
    : `\u4ec5\u7f51\u9875\u767b\u5f55\u5168\u90e8 ${state.accounts.length} \u4e2a\u8d26\u53f7`;
  pushLog("==================================================");
  pushLog(title);
  setStatus("\u7f51\u9875\u767b\u5f55\u4e2d...", "busy");
  try {
    const r = await invoke("web_login_only_command", {
      targetUsername: target,
      target_username: target
    });
    state.results = r.results || [];
    recalculateTotals();
    state.lastFinished = r.finished_at || "-";
    renderMeta();
    renderResults();
    renderTotalBadge();
    pushLog(`\u5b8c\u6210: \u6210\u529f ${r.success_count} / \u5931\u8d25 ${r.fail_count}`);
    pushLog("==================================================");
    setStatus(`\u7f51\u9875\u767b\u5f55\u5b8c\u6210\uff0c\u8017\u65f6 ${Number(r.elapsed_secs || 0).toFixed(2)}s`, "ok");
  } catch (error) {
    setStatus(`\u7f51\u9875\u767b\u5f55\u5931\u8d25: ${toErrorMessage(error)}`, "error");
    pushLog(`\u7f51\u9875\u767b\u5f55\u5931\u8d25: ${toErrorMessage(error)}`);
  } finally {
    state.isRunning = false;
    scheduleStatusRender();
  }
}

// ========== Account Actions ==========

async function copyApiKey(username) {
  const account = state.accounts.find((item) => item.username === username);
  if (!account || !account.api_key) {
    setStatus(`${username} \u672a\u914d\u7f6e API Key`, "warn");
    return;
  }
  await navigator.clipboard.writeText(account.api_key);
  setStatus(`\u5df2\u590d\u5236 ${username} \u7684 API Key`, "ok");
  pushLog(`\u5df2\u590d\u5236 ${username} \u7684 API Key`);
}

async function setClaudeToken(username) {
  try {
    const msg = await invoke("save_claude_token_command", { username });
    await refreshClaudeAccount();
    renderMeta();
    setStatus(msg, "ok");
    pushLog(msg);
  } catch (error) {
    setStatus(`\u8bbe\u7f6e Claude Token \u5931\u8d25: ${toErrorMessage(error)}`, "error");
  }
}

async function setOpenAiToken(username) {
  try {
    const msg = await invoke("save_openai_key_command", { username });
    setStatus(`\u5df2\u8bbe\u7f6e ${username} \u7684 OpenAI Key`, "ok");
    pushLog(msg);
  } catch (error) {
    setStatus(`\u8bbe\u7f6e OpenAI Key \u5931\u8d25: ${toErrorMessage(error)}`, "error");
  }
}

async function deleteAccount(username) {
  if (!confirm(`\u786e\u8ba4\u5220\u9664\u8d26\u53f7 ${username} ?`)) return;
  try {
    const r = await invoke("remove_account_command", { username });
    state.accounts = r.accounts || [];
    state.results = state.results.filter((item) => item.username !== username);
    recalculateTotals();
    if (state.selectedUsername === username) state.selectedUsername = "";
    if (state.editingUsername === username) cancelEdit();
    renderMeta();
    renderAccountSelect();
    renderResults();
    renderTotalBadge();
    renderAccountsTable();
    setStatus(r.message || "\u5220\u9664\u5b8c\u6210", r.success ? "ok" : "warn");
    pushLog(r.message || "\u5220\u9664\u5b8c\u6210");
  } catch (error) {
    setStatus(`\u5220\u9664\u5931\u8d25: ${toErrorMessage(error)}`, "error");
  }
}

async function reloadAccounts() {
  if (state.isRunning) return;
  setStatus("\u6b63\u5728\u91cd\u65b0\u52a0\u8f7d...", "busy");
  try {
    const r = await invoke("reload_accounts_command");
    state.accounts = r.accounts || [];
    syncResultsWithAccounts();
    renderMeta();
    renderAccountSelect();
    renderResults();
    renderTotalBadge();
    renderAccountsTable();
    pushLog(r.message || "\u8d26\u53f7\u91cd\u8f7d\u5b8c\u6210");
    setStatus(r.message || "\u8d26\u53f7\u91cd\u8f7d\u5b8c\u6210", "ok");
  } catch (error) {
    setStatus(`\u91cd\u8f7d\u5931\u8d25: ${toErrorMessage(error)}`, "error");
  }
}

// ========== Auto Mode ==========

function toggleAutoMode() {
  state.autoMode = !state.autoMode;
  refs.btnAuto.textContent = state.autoMode ? "\u505c\u6b62\u81ea\u52a8" : "\u5f00\u542f\u81ea\u52a8";
  if (state.autoMode) {
    state.autoCountdown = 0;
    ensureAutoTimer();
    pushLog(`\u5df2\u5f00\u542f\u81ea\u52a8\u8f6e\u8be2\uff0c\u95f4\u9694 ${state.queryInterval} \u79d2`);
    setStatus("\u5df2\u5f00\u542f\u81ea\u52a8\u8f6e\u8be2", "ok");
  } else {
    clearAutoTimer();
    pushLog("\u5df2\u505c\u6b62\u81ea\u52a8\u8f6e\u8be2");
    setStatus("\u5df2\u505c\u6b62\u81ea\u52a8\u8f6e\u8be2", "ok");
  }
  scheduleStatusRender();
}

function ensureAutoTimer() {
  clearAutoTimer();
  state.autoTimer = setInterval(async () => {
    if (!state.autoMode || state.isRunning) return;
    if (state.autoCountdown <= 0) {
      await runQuery();
      return;
    }
    state.autoCountdown -= 1;
    scheduleStatusRender();
  }, 1000);
}

function clearAutoTimer() {
  if (state.autoTimer) {
    clearInterval(state.autoTimer);
    state.autoTimer = null;
  }
  state.autoCountdown = 0;
}

// ========== Status ==========

function scheduleStatusRender() {
  if (statusRenderPending) return;
  statusRenderPending = true;
  requestAnimationFrame(() => {
    statusRenderPending = false;
    renderStatus();
  });
}

function renderStatus() {
  refs.statusText.textContent = state.statusText;
  refs.statusBar.dataset.state = state.statusState;
  refs.autoLabel.textContent = state.autoMode
    ? `\u81ea\u52a8\u8f6e\u8be2: ${state.autoCountdown}s`
    : "";
  refs.btnQuery.disabled = state.isRunning;
  refs.btnWebLogin.disabled = state.isRunning;
}

function setStatus(text, type) {
  state.statusText = text;
  state.statusState = type || "ok";
  scheduleStatusRender();
}

// ========== Logs ==========

function scheduleLogsRender() {
  if (logsRenderPending) return;
  logsRenderPending = true;
  requestAnimationFrame(() => {
    logsRenderPending = false;
    if (state.progressLogs.length === 0) {
      refs.logsBody.innerHTML = `<div class="logs-empty">\u6682\u65e0\u65e5\u5fd7</div>`;
    } else {
      refs.logsBody.innerHTML = state.progressLogs.map(formatLogLine).join("");
    }
    refs.logCount.textContent = `${state.progressLogs.length} \u6761\u65e5\u5fd7`;
    if (state.activeTab === "logs") {
      refs.logsBody.scrollTop = refs.logsBody.scrollHeight;
    }
  });
}

function formatLogLine(raw) {
  const text = esc(raw);
  // \u5206\u9694\u7ebf
  if (raw.includes("==========")) {
    return `<div class="log-line log-sep"></div>`;
  }
  // \u63d0\u53d6\u65f6\u95f4\u6233
  const match = text.match(/^(\[[^\]]+\])\s(.*)$/);
  const stamp = match ? `<span class="log-ts">${match[1]}</span> ` : "";
  let body = match ? match[2] : text;
  // \u63d0\u53d6\u8d26\u53f7\u6807\u7b7e (\u6765\u81ea\u540e\u7aef\u8fdb\u5ea6\u4e8b\u4ef6\u7684 [username] \u524d\u7f00)
  let accountTag = "";
  const accountMatch = body.match(/^\[([^\]]+)\]\s(.*)$/);
  if (accountMatch) {
    accountTag = `<span class="log-account-tag">[${accountMatch[1]}]</span> `;
    body = accountMatch[2];
  }
  // \u5206\u7c7b\u7740\u8272
  let cls = "";
  if (/\u5931\u8d25|error|\u9519\u8bef|\u4e0d\u53ef\u7528/i.test(raw)) cls = "log-error";
  else if (/\u5b8c\u6210|\u6210\u529f|\u5df2\u590d\u5236|\u5df2\u8bbe\u7f6e|\u5df2\u5f00\u542f|\u5df2\u505c\u6b62|\u521d\u59cb\u5316\u5b8c\u6210/i.test(raw)) cls = "log-ok";
  else if (/\u603b\u4f59\u989d/i.test(raw)) cls = "log-balance";
  else if (/\u67e5\u8be2\u8d26\u53f7|\u67e5\u8be2\u5168\u90e8|\u4ec5\u7f51\u9875\u767b\u5f55|\u5f00\u59cb\u68c0\u67e5|\u5f00\u59cb\u4ec5|\u5f00\u59cb\u67e5\u8be2/i.test(raw)) cls = "log-title";
  return `<div class="log-line ${cls}">${stamp}${accountTag}<span class="log-msg">${body}</span></div>`;
}

function pushLog(message) {
  const now = new Date();
  const stamp = [
    String(now.getHours()).padStart(2, "0"),
    String(now.getMinutes()).padStart(2, "0"),
    String(now.getSeconds()).padStart(2, "0")
  ].join(":");
  state.progressLogs.push(`[${stamp}] ${message}`);
  if (state.progressLogs.length > MAX_PROGRESS_LOGS) {
    state.progressLogs.splice(0, state.progressLogs.length - MAX_PROGRESS_LOGS);
  }
  scheduleLogsRender();
}

// ========== Helpers ==========

function syncResultsWithAccounts() {
  const names = new Set(state.accounts.map((item) => item.username));
  state.results = state.results.filter((item) => names.has(item.username));
  recalculateTotals();
}

function recalculateTotals() {
  let total = 0;
  let count = 0;
  state.results.forEach((item) => {
    if (!item || !item.success) return;
    const v = parseBalance(item.balance_text || "");
    if (v === null) return;
    total += v;
    count += 1;
  });
  state.totalBalance = total;
  state.totalBalanceCount = count;
}

function parseBalance(text) {
  const matched = String(text || "").match(/-?[\d,]+(?:\.\d+)?/);
  if (!matched) return null;
  const v = Number(matched[0].replaceAll(",", ""));
  return Number.isFinite(v) ? v : null;
}

function maskText(text) {
  if (!text || text.length <= 4) return "****";
  return text.slice(0, 2) + "\u00b7".repeat(Math.min(text.length - 4, 8)) + text.slice(-2);
}

function toErrorMessage(error) {
  if (!error) return "\u672a\u77e5\u9519\u8bef";
  if (typeof error === "string") return error;
  if (typeof error === "object" && "message" in error) return String(error.message);
  return String(error);
}

function esc(value) {
  return String(value || "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\"", "&quot;");
}

function escAttr(value) {
  return esc(value).replaceAll("'", "&#39;");
}

function el(id) {
  return document.getElementById(id);
}

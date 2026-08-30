import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import "./style.css";

type Device = { id: string; name: string; status: "connected" | "connecting" | "disconnected"; autoConnect: boolean };
type Settings = { minimizeToTray: boolean; autoConnect: boolean; connectionNotifications: boolean; launchAtLogin: boolean; startMinimized: boolean; theme: "system" | "light" | "dark" };
type Snapshot = { devices: Device[]; settings: Settings; lastDeviceId?: string; autoConnectDeviceIds: string[]; displayLanguage: "ja" | "en" };
type ConnectionEvent = { deviceId: string; name: string; status: "connected" | "disconnected" };
type DeviceFilter = "all" | "connected" | "disconnected";

const quickMode = new URLSearchParams(window.location.search).get("mode") === "quick";
const initialLanguage: Snapshot["displayLanguage"] = navigator.language.toLowerCase().startsWith("ja") ? "ja" : "en";
let snapshot: Snapshot = { devices: [], settings: { minimizeToTray: true, autoConnect: false, connectionNotifications: false, launchAtLogin: false, startMinimized: true, theme: "system" }, autoConnectDeviceIds: [], displayLanguage: initialLanguage };
let languageOverride: Snapshot["displayLanguage"] | null = null;
const displayLanguage = () => languageOverride ?? snapshot.displayLanguage;
const tr = (ja: string, en: string) => displayLanguage() === "ja" ? ja : en;
let progress = tr("Bluetooth デバイスを検索してください。", "Search for Bluetooth devices to get started.");
let logOpen = false;
let logContent = "";
let diagnosticsOpen = false;
let diagnosticsContent = "";
let appVersion = "";
let deviceFilter: DeviceFilter = "all";
let deviceSearch = "";
const deviceMessages = new Map<string, string>();

function systemDark() { return window.matchMedia("(prefers-color-scheme: dark)").matches; }
function applyTheme() {
  document.documentElement.lang = displayLanguage();
  document.documentElement.dataset.theme = snapshot.settings.theme === "system" ? (systemDark() ? "dark" : "light") : snapshot.settings.theme;
}
function statusText(status: Device["status"]) {
  return displayLanguage() === "ja"
    ? ({ connected: "接続中", connecting: "接続しています…", disconnected: "未接続" })[status]
    : ({ connected: "Connected", connecting: "Connecting…", disconnected: "Disconnected" })[status];
}
function sortedDevices() {
  const rank = (device: Device) => device.status === "connected" ? 0 : device.status === "connecting" ? 1 : device.autoConnect ? 2 : snapshot.lastDeviceId === device.id ? 3 : 4;
  return [...snapshot.devices].sort((left, right) => rank(left) - rank(right) || left.name.localeCompare(right.name, displayLanguage()));
}
function diagnosticsChecklist() {
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const connecting = snapshot.devices.filter((device) => device.status === "connecting").length;
  const targets = snapshot.autoConnectDeviceIds.length;
  const item = (ok: boolean, title: string, detail: string) => `<li class="diagnostic-item ${ok ? "ok" : "attention"}"><span aria-hidden="true">${ok ? "✓" : "!"}</span><div><strong>${escapeHtml(title)}</strong><p>${escapeHtml(detail)}</p></div></li>`;
  return `<ul class="diagnostic-list">
    ${item(snapshot.devices.length > 0, tr("音声受信に利用できる端末", "Devices available for audio reception"), snapshot.devices.length > 0 ? tr(`${snapshot.devices.length}台見つかっています。`, `${snapshot.devices.length} device(s) found.`) : tr("ペアリングを確認してから再検索してください。", "Check pairing, then refresh the device list."))}
    ${item(connected > 0 || connecting > 0, tr("現在の接続", "Current connections"), connecting > 0 ? tr(`${connected}台接続中、${connecting}台接続処理中です。`, `${connected} connected and ${connecting} connecting.`) : connected > 0 ? tr(`${connected}台接続中です。`, `${connected} connected.`) : tr("現在アプリが保持している接続はありません。", "The app is not currently holding a connection."))}
    ${item(!snapshot.settings.autoConnect || targets > 0 || !!snapshot.lastDeviceId, tr("起動時の自動接続", "Startup auto-connect"), snapshot.settings.autoConnect ? targets > 0 ? tr(`${targets}台が対象です。`, `${targets} device(s) selected.`) : tr("最後に接続した端末が対象になります。", "The last connected device will be used.") : tr("無効です。必要な場合は通常画面の設定で有効にできます。", "Disabled. Enable it in the main window if needed."))}
  </ul><p class="diagnostic-note">${tr("スマートフォン側やWindows側から切断した場合、表示は自動更新されません。対象端末を一度「切断」してから再接続してください。", "If the phone or Windows disconnects externally, the display is not updated automatically. Select Disconnect for that device before reconnecting.")}</p>`;
}
function diagnosticsDialog() {
  return `<div class="log-backdrop" role="presentation"><section class="log-dialog diagnostics-dialog" role="dialog" aria-modal="true" aria-labelledby="diagnostics-title"><div class="log-heading"><div><h2 id="diagnostics-title">${tr("セットアップ診断", "Setup diagnostics")}</h2><p>${tr("端末の検出状況と設定を確認します。", "Check device discovery and startup settings.")}</p></div><button class="ghost" id="close-diagnostics" aria-label="${tr("診断情報を閉じる", "Close diagnostics")}">${tr("閉じる", "Close")}</button></div>${diagnosticsChecklist()}<details class="technical-details"><summary>${tr("技術情報", "Technical details")}</summary><pre class="log-content diagnostics-content">${escapeHtml(diagnosticsContent)}</pre></details></section></div>`;
}
function renderQuick() {
  const activeId = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
  applyTheme();
  document.body.classList.add("quick-mode");
  document.body.classList.toggle("modal-open", diagnosticsOpen);
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const query = deviceSearch.trim().toLocaleLowerCase();
  const devices = sortedDevices().filter((device) => {
    const matchesFilter = deviceFilter === "all" || (deviceFilter === "connected" ? device.status !== "disconnected" : device.status === "disconnected");
    return matchesFilter && (!query || device.name.toLocaleLowerCase().includes(query));
  });
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `<main class="quick-shell">
    <header class="quick-header"><div class="brand"><img src="/icon.svg" aria-hidden="true"/><div><strong>Phone Audio Receiver</strong><span>${tr("クイック操作", "Quick controls")}</span></div></div><button class="ghost quick-close" id="close-quick" aria-label="${tr("閉じる", "Close")}">×</button></header>
    <section class="quick-summary"><div><strong>${connected}</strong><span>${tr("台接続中", "connected")}</span></div><button class="secondary" id="quick-refresh">${tr("再検索", "Refresh")}</button></section>
    <div class="quick-tools"><input id="device-search" type="search" value="${escapeHtml(deviceSearch)}" placeholder="${tr("端末名を検索", "Search devices")}" aria-label="${tr("端末名を検索", "Search devices")}"/><div class="filter-group" role="group" aria-label="${tr("端末の絞り込み", "Filter devices")}">${(["all", "connected", "disconnected"] as DeviceFilter[]).map((filter) => `<button class="filter-button ${deviceFilter === filter ? "active" : ""}" data-filter="${filter}">${filter === "all" ? tr("すべて", "All") : filter === "connected" ? tr("接続中", "Connected") : tr("未接続", "Disconnected")}</button>`).join("")}</div></div>
    <section class="quick-devices" role="list">${devices.length ? devices.map((device) => `<article class="device quick-device ${device.status}" role="listitem"><span class="dot ${device.status}"></span><div class="device-name"><strong>${escapeHtml(device.name)}</strong><span>${statusText(device.status)}${device.autoConnect ? ` · ${tr("自動接続", "Auto-connect")}` : ""}</span>${deviceMessages.has(device.id) ? `<small>${escapeHtml(deviceMessages.get(device.id)!)}</small>` : ""}</div><button class="${device.status === "disconnected" ? "primary" : "secondary"}" data-device="${encodeURIComponent(device.id)}" data-action="${device.status === "disconnected" ? "connect" : "disconnect"}">${device.status === "disconnected" ? tr("接続", "Connect") : device.status === "connecting" ? tr("中止", "Cancel") : tr("切断", "Disconnect")}</button></article>`).join("") : `<div class="empty quick-empty"><strong>${tr("該当する端末がありません", "No matching devices")}</strong><span>${tr("検索条件を変更するか、再検索してください。", "Change the filter or refresh the device list.")}</span></div>`}</section>
    <footer class="quick-footer"><span>${tr(`全${snapshot.devices.length}台`, `${snapshot.devices.length} total`)}</span><div class="quick-footer-actions"><button class="ghost" id="quick-diagnostics">${tr("セットアップ診断", "Setup diagnostics")}</button><button class="secondary" id="open-main-window">${tr("詳細画面を開く", "Open details")}</button></div></footer>
    ${diagnosticsOpen ? diagnosticsDialog() : ""}
  </main>`;
  if (activeId) document.getElementById(activeId)?.focus();
  document.querySelector("#close-quick")!.addEventListener("click", () => invoke("hide_quick_window"));
  document.querySelector("#quick-refresh")!.addEventListener("click", refresh);
  document.querySelector("#quick-diagnostics")!.addEventListener("click", showDiagnostics);
  document.querySelector("#open-main-window")!.addEventListener("click", () => invoke("switch_to_main_window"));
  document.querySelector("#close-diagnostics")?.addEventListener("click", () => { diagnosticsOpen = false; render(); });
  document.querySelector<HTMLInputElement>("#device-search")!.addEventListener("input", (event) => { deviceSearch = (event.target as HTMLInputElement).value; render(); });
  document.querySelectorAll<HTMLButtonElement>("[data-filter]").forEach((button) => button.addEventListener("click", () => { deviceFilter = button.dataset.filter as DeviceFilter; render(); }));
  bindDeviceActions();
}
function render() {
  if (quickMode) {
    renderQuick();
    return;
  }
  const previousLogScroll = document.querySelector<HTMLElement>(".log-content")?.scrollTop;
  const activeId = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
  applyTheme();
  // モーダル表示中は、ログ本文以外でのホイール操作が背後の画面へ届かないようにする。
  document.body.classList.toggle("modal-open", logOpen || diagnosticsOpen);
  document.documentElement.classList.toggle("modal-open", logOpen || diagnosticsOpen);
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const mainDevices = sortedDevices();
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
    <main class="shell">
      <header class="topbar">
        <div class="brand"><img src="/icon.svg" aria-hidden="true"/><span>Phone Audio Receiver</span></div>
        <div class="actions"><button class="ghost language-toggle" id="toggle-language" title="${tr("英語表示に切り替える", "Switch to Japanese")}" aria-label="${tr("英語表示に切り替える", "Switch to Japanese")}"><span aria-hidden="true">文/A</span><strong>${tr("日本語", "English")}</strong></button><button class="ghost" id="open-quick-window">${tr("クイック表示", "Quick view")}</button><button class="ghost" id="open-log">${tr("ログ", "Log")}</button><button class="ghost" id="show-diagnostics">${tr("セットアップ診断", "Setup diagnostics")}</button></div>
      </header>
      <section class="panel devices-panel">
        <div class="panel-heading"><div><div class="devices-title"><h2>${tr("接続端末", "Devices")}</h2><span class="connection-count" aria-live="polite"><strong>${connected}</strong>${tr("台接続中", "connected")}</span></div><p id="connection-progress">${escapeHtml(progress)}</p></div><button class="primary refresh-button" id="refresh">${tr("再検索", "Refresh")}</button></div>
        <div class="devices" role="list">${mainDevices.length ? mainDevices.map((device) => `
          <article class="device ${device.status}" role="listitem">
            <div class="device-icon">♬</div><div class="device-name"><strong>${escapeHtml(device.name)}</strong><span>${statusText(device.status)}</span></div>
            <span class="dot ${device.status}"></span>
            <button class="device-auto-connect ${device.autoConnect ? "active" : ""}" data-auto-connect="${encodeURIComponent(device.id)}" aria-pressed="${device.autoConnect}" ${snapshot.settings.autoConnect ? "" : "disabled"} title="${snapshot.settings.autoConnect ? tr("この端末を起動時の自動接続対象にする", "Automatically connect to this device at startup") : tr("設定で「起動時に選択した端末へ自動接続する」をオンにすると選択できます", "Enable startup auto-connect in Settings before selecting devices")}">${tr("自動接続", "Auto-connect")}</button>
            <button class="${device.status === "disconnected" ? "primary" : "secondary"}" data-device="${encodeURIComponent(device.id)}" data-action="${device.status === "disconnected" ? "connect" : "disconnect"}">${device.status === "disconnected" ? tr("接続", "Connect") : device.status === "connecting" ? tr("中止", "Cancel") : tr("切断", "Disconnect")}</button>
          </article>`).join("") : `<div class="empty"><strong>${tr("まだ端末がありません", "No devices found yet")}</strong><span>${tr("「再検索」を押すと、音声受信に利用できるペア済みのBluetooth端末を表示します。", "Select Refresh to show paired Bluetooth devices available for audio reception.")}</span></div>`}</div>
      </section>
      <section class="panel settings"><div class="panel-heading"><div><h2>${tr("設定", "Settings")}</h2><p>${tr("起動時の動作と表示を調整できます。", "Configure startup behavior and appearance.")}</p></div></div>
        <div class="settings-grid">
          ${checkbox("autoConnect", tr("起動時に選択した端末へ自動接続する", "Connect to selected devices at startup"), snapshot.settings.autoConnect)}
          ${checkbox("connectionNotifications", tr("接続状態をWindows通知で知らせる", "Show Windows notifications for connection changes"), snapshot.settings.connectionNotifications)}
          ${checkbox("launchAtLogin", tr("Windows 起動時にアプリを起動する", "Launch the app when signing in to Windows"), snapshot.settings.launchAtLogin)}
          ${checkbox("startMinimized", tr("Windows 起動時は最小化して開始する", "Start minimized when launched with Windows"), snapshot.settings.startMinimized)}
          ${checkbox("minimizeToTray", tr("閉じるときにタスクトレイへ格納する", "Minimize to the system tray when closing"), snapshot.settings.minimizeToTray)}
          <label class="theme-control"><span>${tr("テーマ", "Theme")}</span><select id="theme"><option value="system">${tr("システムに合わせる", "Use system setting")}</option><option value="dark">${tr("ダーク", "Dark")}</option><option value="light">${tr("ライト", "Light")}</option></select></label>
        </div>
      </section>
      <footer>Phone Audio Receiver${appVersion ? ` <span>v${escapeHtml(appVersion)}</span>` : ""}</footer>
      ${logOpen ? `<div class="log-backdrop" role="presentation"><section class="log-dialog" role="dialog" aria-modal="true" aria-labelledby="log-title"><div class="log-heading"><div><h2 id="log-title">${tr("アプリログ", "Application log")}</h2><p>${tr("直近30行を匿名化して表示しています。日時はJST（+09:00）です。", "Showing the latest 30 lines with personal data anonymized. Times are UTC (Z).")}</p></div><button class="ghost" id="close-log" aria-label="${tr("ログを閉じる", "Close log")}">${tr("閉じる", "Close")}</button></div><pre class="log-content">${escapeHtml(formatLog(logContent) || tr("ログはまだありません。", "No log entries yet."))}</pre><div class="log-actions"><button class="secondary" id="refresh-log">${tr("更新", "Refresh")}</button><button class="secondary" id="copy-log">${tr("匿名化済みログをコピー", "Copy anonymized log")}</button><button class="ghost" id="open-log-folder" title="${tr("生ログには端末名、Bluetooth ID、ユーザー環境のパスが含まれる場合があります", "Raw logs may contain device names, Bluetooth IDs, and paths from the user environment")}">${tr("生ログフォルダーを開く", "Open raw log folder")}</button></div></section></div>` : ""}
      ${diagnosticsOpen ? diagnosticsDialog() : ""}
    </main>`;
  const select = document.querySelector<HTMLSelectElement>("#theme")!; select.value = snapshot.settings.theme;
  if (previousLogScroll !== undefined) document.querySelector<HTMLElement>(".log-content")?.scrollTo({ top: previousLogScroll });
  if (activeId) document.getElementById(activeId)?.focus();
  document.querySelector("#refresh")!.addEventListener("click", refresh);
  document.querySelector("#toggle-language")!.addEventListener("click", toggleLanguage);
  document.querySelector("#open-quick-window")!.addEventListener("click", () => invoke("switch_to_quick_window"));
  document.querySelector("#open-log")!.addEventListener("click", showLog);
  document.querySelector("#show-diagnostics")!.addEventListener("click", showDiagnostics);
  document.querySelector("#close-log")?.addEventListener("click", () => { logOpen = false; render(); });
  document.querySelector("#close-diagnostics")?.addEventListener("click", () => { diagnosticsOpen = false; render(); });
  document.querySelector("#refresh-log")?.addEventListener("click", showLog);
  document.querySelector("#copy-log")?.addEventListener("click", copyLog);
  document.querySelector("#open-log-folder")?.addEventListener("click", () => invoke("open_log_folder"));
  bindDeviceActions();
  document.querySelectorAll<HTMLButtonElement>("[data-auto-connect]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.autoConnect!);
    try {
      await invoke("set_device_auto_connect", { deviceId: id, enabled: !button.classList.contains("active") });
      setProgress(tr("端末ごとの自動接続設定を保存しました。", "Device auto-connect setting saved."));
    } catch (error) { setProgress(tr(`設定を保存できませんでした: ${error}`, `Could not save the setting: ${error}`)); }
  }));
  document.querySelectorAll<HTMLInputElement>("input[data-setting]").forEach((input) => input.addEventListener("change", saveSettings));
  select.addEventListener("change", saveSettings);
}
function bindDeviceActions() {
  document.querySelectorAll<HTMLButtonElement>("[data-device]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.device!);
    try { await invoke(button.dataset.action === "connect" ? "connect_device" : "disconnect_device", { deviceId: id }); }
    catch (error) {
      deviceMessages.set(id, String(error));
      setProgress(tr(`操作に失敗しました: ${error}`, `The operation failed: ${error}`));
      if (quickMode) render();
    }
  }));
}
function checkbox(key: keyof Settings, label: string, checked: boolean, disabled = false) { return `<label class="check"><input type="checkbox" data-setting="${key}" ${checked ? "checked" : ""} ${disabled ? "disabled" : ""}/><span>${label}</span></label>`; }
function escapeHtml(value: string) { const node = document.createElement("span"); node.textContent = value; return node.innerHTML; }
function setProgress(message: string) {
  progress = message;
  const element = document.querySelector<HTMLElement>("#connection-progress");
  if (element) element.textContent = message;
}
function formatLog(value: string) {
  return value.split("\n").map((line) => line.replace(/^\[(\d+)\]\s?(.*)$/, (_, epoch, message) => `[${formatLogTimestamp(Number(epoch))}] ${message}`)).join("\n");
}
function formatLogTimestamp(epochSeconds: number) {
  const date = new Date(epochSeconds * 1000);
  if (Number.isNaN(date.getTime())) return String(epochSeconds);
  if (displayLanguage() === "en") return date.toISOString().replace(".000Z", "Z");
  // UTC+09:00 is Japan Standard Time and has no daylight-saving adjustment.
  const jst = new Date(date.getTime() + 9 * 60 * 60 * 1000);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${jst.getUTCFullYear()}-${pad(jst.getUTCMonth() + 1)}-${pad(jst.getUTCDate())} ${pad(jst.getUTCHours())}:${pad(jst.getUTCMinutes())}:${pad(jst.getUTCSeconds())}+09:00`;
}
function toggleLanguage() {
  languageOverride = displayLanguage() === "ja" ? "en" : "ja";
  progress = tr("表示言語を日本語に切り替えました。", "Display language changed to English.");
  render();
}
async function refresh() { setProgress(tr("Bluetooth デバイスを検索しています…", "Searching for Bluetooth devices…")); try { snapshot = await invoke<Snapshot>("refresh_devices"); progress = snapshot.devices.length ? tr(`${snapshot.devices.length} 台の端末を見つけました。`, `Found ${snapshot.devices.length} device${snapshot.devices.length === 1 ? "" : "s"}.`) : tr("利用できるペア済み端末は見つかりませんでした。", "No paired devices are currently available."); } catch (error) { progress = tr(`検索できませんでした: ${error}`, `Could not search for devices: ${error}`); } render(); }
async function saveSettings() {
  const setting = (key: keyof Settings) => document.querySelector<HTMLInputElement>(`[data-setting="${key}"]`)?.checked ?? snapshot.settings[key];
  const notificationsEnabled = setting("connectionNotifications") as boolean;
  snapshot.settings = { ...snapshot.settings, autoConnect: setting("autoConnect") as boolean, connectionNotifications: notificationsEnabled, launchAtLogin: setting("launchAtLogin") as boolean, startMinimized: setting("startMinimized") as boolean, minimizeToTray: setting("minimizeToTray") as boolean, theme: document.querySelector<HTMLSelectElement>("#theme")!.value as Settings["theme"] };
  try {
    if (notificationsEnabled && !await ensureNotificationPermission()) {
      snapshot.settings.connectionNotifications = false;
      progress = tr("Windows通知の許可がないため、通知設定は有効にできませんでした。", "Notifications could not be enabled because Windows permission was not granted.");
    }
    snapshot.settings = await invoke<Settings>("save_settings", { settings: snapshot.settings });
    if (!progress.startsWith("Windows通知") && !progress.startsWith("Notifications")) progress = tr("設定を保存しました。", "Settings saved.");
  } catch (error) { progress = tr(`設定を保存できませんでした: ${error}`, `Could not save settings: ${error}`); }
  render();
}
async function showDiagnostics() {
  try {
    diagnosticsContent = await invoke<string>("diagnostics");
    diagnosticsOpen = true;
    render();
  }
  catch (error) { setProgress(tr(`診断情報を取得できませんでした: ${error}`, `Could not retrieve diagnostics: ${error}`)); }
}
async function showLog() {
  logOpen = true;
  logContent = tr("読み込んでいます…", "Loading…");
  render();
  try { logContent = await invoke<string>("recent_log"); }
  catch (error) { logContent = tr(`ログを読み込めませんでした: ${error}`, `Could not load the log: ${error}`); }
  render();
}
async function copyLog() {
  try {
    await navigator.clipboard.writeText(formatLog(logContent));
    setProgress(tr("匿名化済みログをクリップボードへコピーしました。", "Anonymized log copied to the clipboard."));
  } catch (error) {
    setProgress(tr(`匿名化済みログをコピーできませんでした: ${error}`, `Could not copy the anonymized log: ${error}`));
  }
}
async function ensureNotificationPermission() {
  if (await isPermissionGranted()) return true;
  return await requestPermission() === "granted";
}
async function notifyConnection(event: ConnectionEvent) {
  if (!snapshot.settings.connectionNotifications || !await ensureNotificationPermission()) return;
  const body = event.status === "connected"
    ? tr(`${event.name} を接続しました。`, `${event.name} connected.`)
    : tr(`${event.name} との接続が切れました。`, `${event.name} disconnected.`);
  sendNotification({ title: "Phone Audio Receiver", body });
}
function renderLoading() {
  document.documentElement.dataset.theme = systemDark() ? "dark" : "light";
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `<main class="shell"><section class="loading"><img src="/icon.svg" aria-hidden="true"/><strong>Phone Audio Receiver</strong><span>${tr("設定を読み込んでいます…", "Loading settings…")}</span></section></main>`;
}
async function start() {
  // 保存済みテーマを取得してから本画面を描画し、テーマの切り替わりを防ぐ。
  renderLoading();
  try {
    appVersion = await getVersion();
    await listen<Snapshot>("app-state-changed", (event) => { snapshot = event.payload; render(); });
    await listen<{ deviceId: string; message: string }>("connection-progress", (event) => {
      deviceMessages.set(event.payload.deviceId, event.payload.message);
      setProgress(event.payload.message);
      if (quickMode) render();
    });
    await listen<ConnectionEvent>("connection-state-changed", (event) => {
      void notifyConnection(event.payload);
    });
    snapshot = await invoke<Snapshot>("app_snapshot");
    render();
    if (!quickMode) await refresh();
    if (!quickMode && snapshot.settings.autoConnect) {
      const targets = snapshot.autoConnectDeviceIds.length ? snapshot.autoConnectDeviceIds : snapshot.lastDeviceId ? [snapshot.lastDeviceId] : [];
      for (const device of snapshot.devices.filter((candidate) => targets.includes(candidate.id) && candidate.status === "disconnected")) {
        await invoke("connect_device", { deviceId: device.id });
      }
    }
  } catch (error) {
    progress = tr(`初期化できませんでした: ${error}`, `Initialization failed: ${error}`);
    render();
  }
}
window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => { if (snapshot.settings.theme === "system") applyTheme(); });
void start();

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import "./style.css";

type Device = { id: string; name: string; systemName: string; status: "connected" | "connecting" | "disconnected"; autoConnect: boolean; disconnectTimerDeadline?: number; favorite: boolean; trayPinned: boolean; orderIndex?: number };
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
let shortcutsOpen = false;
let moreMenuOpen = false;
let openSelectId: string | null = null;
let appVersion = "";
let updateStatus: "idle" | "checking" | "current" | "available" | "error" = "idle";
let latestVersion = "";
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
function uiIcon(name: "audio" | "favorite" | "pin" | "up" | "down" | "edit" | "clock" | "theme") {
  const paths = {
    audio: '<path d="M11 5 6.5 9H3v6h3.5l4.5 4V5Z"/><path d="M15 9.5a4 4 0 0 1 0 5M17.5 7a7.5 7.5 0 0 1 0 10"/>',
    favorite: '<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-2.9-5.6 2.9 1.1-6.2L3 9.6l6.2-.9L12 3Z"/>',
    pin: '<path d="M9 4h6l-.8 5 2.8 3H7l2.8-3L9 4Z"/><path d="M12 12v8"/>',
    up: '<path d="m6 14 6-6 6 6"/>',
    down: '<path d="m6 10 6 6 6-6"/>',
    edit: '<path d="m14.7 5.3 4 4M4 20l4.2-1 10.5-10.5a2.8 2.8 0 0 0-4-4L4.2 15 4 20Z"/>',
    clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
    theme: '<circle cx="12" cy="12" r="3.5"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M18.7 5.3l-1.4 1.4M6.7 17.3l-1.4 1.4"/>',
  };
  return `<svg class="ui-icon" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${paths[name]}</svg>`;
}
function sortedDevices() {
  const collator = new Intl.Collator(displayLanguage(), { sensitivity: "base" });
  return [...snapshot.devices].sort((left, right) => {
    const order = (left.orderIndex ?? Number.MAX_SAFE_INTEGER) - (right.orderIndex ?? Number.MAX_SAFE_INTEGER);
    return order || collator.compare(left.name, right.name) || left.id.localeCompare(right.id);
  });
}
function timerRemaining(deadline: number) {
  const seconds = Math.max(0, deadline - Math.floor(Date.now() / 1000));
  if (seconds === 0) return tr("まもなく切断", "Disconnecting soon");
  const minutes = Math.floor(seconds / 60);
  const remainder = String(seconds % 60).padStart(2, "0");
  return tr(`残り ${minutes}:${remainder}`, `${minutes}:${remainder} left`);
}
function disconnectTimerControl(device: Device) {
  const encodedId = encodeURIComponent(device.id);
  const disabled = device.status !== "connected" ? "disabled" : "";
  const selectId = `timer:${device.id}`;
  const encodedSelectId = encodeURIComponent(selectId);
  const open = openSelectId === selectId;
  const value = device.disconnectTimerDeadline
    ? `<span class="custom-select-value" data-timer-deadline="${device.disconnectTimerDeadline}">${timerRemaining(device.disconnectTimerDeadline)}</span>`
    : `<span class="custom-select-value">${tr("切断タイマー", "Disconnect timer")}</span>`;
  const cancel = device.disconnectTimerDeadline ? `<button type="button" role="option" aria-selected="false" data-timer-option="0" data-timer-device="${encodedId}">${tr("タイマー解除", "Cancel timer")}</button>` : "";
  const option = (minutes: number, ja: string, en: string) => `<button type="button" role="option" aria-selected="false" data-timer-option="${minutes}" data-timer-device="${encodedId}">${tr(ja, en)}</button>`;
  return `<div class="custom-select timer-select-control ${open ? "open" : ""}"><button type="button" class="custom-select-trigger" id="timer-trigger-${encodedId}" data-select-toggle="${encodedSelectId}" aria-label="${escapeHtml(tr(`${device.name} の切断タイマー`, `Disconnect timer for ${device.name}`))}" aria-haspopup="listbox" aria-expanded="${open}" aria-controls="timer-options-${encodedId}" ${disabled}><span class="select-leading" aria-hidden="true">${uiIcon("clock")}</span>${value}<span class="custom-select-chevron" aria-hidden="true"></span></button>${open ? `<div class="custom-select-menu" id="timer-options-${encodedId}" role="listbox">${cancel}${option(15, "15分", "15 min")}${option(30, "30分", "30 min")}${option(60, "1時間", "1 hour")}${option(120, "2時間", "2 hours")}</div>` : ""}</div>`;
}
function themeControl() {
  const selectId = "theme";
  const open = openSelectId === selectId;
  const labels: Record<Settings["theme"], string> = {
    system: tr("システムに合わせる", "Use system setting"),
    dark: tr("ダーク", "Dark"),
    light: tr("ライト", "Light"),
  };
  const option = (value: Settings["theme"]) => `<button type="button" role="option" aria-selected="${snapshot.settings.theme === value}" data-theme-option="${value}"><span class="custom-option-check" aria-hidden="true">${snapshot.settings.theme === value ? "✓" : ""}</span><span>${labels[value]}</span></button>`;
  return `<div class="custom-select theme-select-control ${open ? "open" : ""}"><button type="button" class="custom-select-trigger" id="theme" data-select-toggle="theme" data-theme-value="${snapshot.settings.theme}" aria-label="${tr("テーマ", "Theme")}" aria-haspopup="listbox" aria-expanded="${open}" aria-controls="theme-options"><span class="select-leading" aria-hidden="true">${uiIcon("theme")}</span><span class="custom-select-value">${labels[snapshot.settings.theme]}</span><span class="custom-select-chevron" aria-hidden="true"></span></button>${open ? `<div class="custom-select-menu" id="theme-options" role="listbox">${option("system")}${option("dark")}${option("light")}</div>` : ""}</div>`;
}
function mainConnectionButton(device: Device) {
  const action = device.status === "disconnected" ? "connect" : "disconnect";
  const kind = device.status === "disconnected" ? "connect" : device.status === "connecting" ? "cancel" : "disconnect";
  const label = device.status === "disconnected" ? tr("接続", "Connect") : device.status === "connecting" ? tr("中止", "Cancel") : tr("切断", "Disconnect");
  return `<button class="app-action device-connect-action ${kind}-action" data-device="${encodeURIComponent(device.id)}" data-action="${action}"><span>${label}</span></button>`;
}
function updateTimerCountdowns() {
  document.querySelectorAll<HTMLElement>("[data-timer-deadline]").forEach((option) => {
    option.textContent = timerRemaining(Number(option.dataset.timerDeadline));
  });
}
function deviceNameMarkup(device: Device) {
  const original = device.name !== device.systemName ? `<small>${escapeHtml(device.systemName)}</small>` : "";
  return `<strong>${escapeHtml(device.name)}</strong>${original}`;
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
  return `<div class="log-backdrop" role="presentation"><section class="log-dialog diagnostics-dialog" role="dialog" aria-modal="true" aria-labelledby="diagnostics-title"><div class="log-heading"><div><h2 id="diagnostics-title">${tr("セットアップ診断", "Setup diagnostics")}</h2><p>${tr("端末の検出状況と設定を確認し、Windowsの関連設定を開けます。", "Check device discovery and open related Windows settings.")}</p></div><button class="ghost" id="close-diagnostics" aria-label="${tr("診断情報を閉じる", "Close diagnostics")}">${tr("閉じる", "Close")}</button></div>${diagnosticsChecklist()}<div class="diagnostic-actions"><button class="secondary" id="open-bluetooth-settings">${tr("Bluetooth設定を開く", "Open Bluetooth settings")}</button><button class="secondary" id="open-sound-settings">${tr("サウンド設定を開く", "Open sound settings")}</button><button class="ghost" id="copy-diagnostics">${tr("匿名の診断概要をコピー", "Copy anonymous summary")}</button></div><details class="technical-details"><summary>${tr("技術情報", "Technical details")}</summary><pre class="log-content diagnostics-content">${escapeHtml(diagnosticsContent)}</pre></details></section></div>`;
}
function shortcutsDialog() {
  const shortcut = (keys: string, ja: string, en: string) => `<li><kbd>${escapeHtml(keys)}</kbd><span>${escapeHtml(tr(ja, en))}</span></li>`;
  const iconHelp = (icon: string, jaTitle: string, enTitle: string, jaDescription: string, enDescription: string) => `<li><span class="reference-icon" aria-hidden="true">${icon}</span><span><strong>${tr(jaTitle, enTitle)}</strong><small>${tr(jaDescription, enDescription)}</small></span></li>`;
  return `<div class="log-backdrop" role="presentation"><section class="log-dialog shortcuts-dialog" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title"><div class="log-heading"><div><h2 id="shortcuts-title">${tr("操作方法", "Keyboard and controls")}</h2><p>${tr("端末カードのアイコンとキーボード操作を確認できます。", "Reference for device-card icons and keyboard controls.")}</p></div><button class="ghost" id="close-shortcuts" aria-label="${tr("操作方法を閉じる", "Close keyboard reference")}">${tr("閉じる", "Close")}</button></div><div class="reference-content">
    <h3>${tr("アイコンと機能", "Icons and features")}</h3><ul class="icon-reference-list">
      ${iconHelp(uiIcon("audio"), "音声端末", "Audio device", "音声を受信できるペアリング済み端末です。", "A paired device available for audio reception.")}
      ${iconHelp(uiIcon("favorite"), "お気に入り", "Favorite", "タスクトレイで優先的に表示します。", "Prioritizes the device in the system tray.")}
      ${iconHelp(uiIcon("pin"), "トレイ固定", "Tray pin", "タスクトレイの先頭グループへ固定します。", "Pins the device to the first group in the system tray.")}
      ${iconHelp(uiIcon("edit"), "表示名を変更", "Rename", "Windowsの名前を変えず、アプリ内の表示名だけを変更します。", "Changes only the name shown in this app, not the Windows name.")}
      ${iconHelp(`<span class="reference-icon-pair">${uiIcon("up")}${uiIcon("down")}</span>`, "表示順", "Display order", "通常画面、クイック画面、トレイで使う順番を変更します。", "Changes the order used in the main window, quick controls, and tray.")}
      ${iconHelp(uiIcon("clock"), "切断タイマー", "Disconnect timer", "選択した時間が経過すると、その端末だけを切断します。", "Disconnects only that device after the selected time.")}
      ${iconHelp(uiIcon("theme"), "テーマ", "Theme", "システム、ダーク、ライトから配色を選択します。", "Selects system, dark, or light appearance.")}
      ${iconHelp("<strong>…</strong>", "その他", "More", "操作方法、診断、ログ、アップデート確認を開きます。", "Opens controls, diagnostics, logs, and update checking.")}
    </ul>
    <h3>${tr("キーボード操作", "Keyboard controls")}</h3><ul class="shortcut-list">
      ${shortcut("F5", "端末を再検索", "Refresh the device list")}
      ${shortcut("Alt + Q", "通常画面とクイック操作画面を切り替え", "Switch between the main and quick-controls windows")}
      ${shortcut("Ctrl + F", "クイック操作画面の端末検索欄へ移動", "Focus device search in quick controls")}
      ${shortcut("Esc", "開いているメニュー、ダイアログまたはクイック操作画面を閉じる", "Close the active menu, dialog, or quick-controls window")}
      ${shortcut("Tab / Shift + Tab", "操作項目を順方向／逆方向へ移動", "Move forward or backward through controls")}
    </ul><p class="shortcut-note">${tr("起動時に自動接続する端末は、カード下段のスイッチで選択します。先頭端末の上移動と末尾端末の下移動は使用できません。", "Use the switch on the lower row of each card to select startup auto-connect devices. Move up is unavailable for the first device, and move down is unavailable for the last device.")}</p>
  </div></section></div>`;
}
function logDialog() {
  return `<div class="log-backdrop" role="presentation"><section class="log-dialog" role="dialog" aria-modal="true" aria-labelledby="log-title"><div class="log-heading"><div><h2 id="log-title">${tr("アプリログ", "Application log")}</h2><p>${tr("直近30行を匿名化して表示しています。日時はJST（+09:00）です。", "Showing the latest 30 lines with personal data anonymized. Times are UTC (Z).")}</p></div><button class="ghost" id="close-log" aria-label="${tr("ログを閉じる", "Close log")}">${tr("閉じる", "Close")}</button></div><pre class="log-content">${escapeHtml(formatLog(logContent) || tr("ログはまだありません。", "No log entries yet."))}</pre><div class="log-actions"><button class="secondary" id="refresh-log">${tr("更新", "Refresh")}</button><button class="secondary" id="copy-log">${tr("匿名化済みログをコピー", "Copy anonymized log")}</button><button class="ghost" id="open-log-folder" title="${tr("生ログには端末名、Bluetooth ID、ユーザー環境のパスが含まれる場合があります", "Raw logs may contain device names, Bluetooth IDs, and paths from the user environment")}">${tr("生ログフォルダーを開く", "Open raw log folder")}</button></div></section></div>`;
}
function overflowMenu() {
  const updateLabel = updateStatus === "checking"
    ? tr("最新版を確認中…", "Checking for updates…")
    : updateStatus === "available"
      ? tr(`最新版 v${latestVersion}`, `Version ${latestVersion} available`)
      : tr("アップデートを確認", "Check for updates");
  return `<div class="overflow-actions"><button type="button" class="ghost more-actions-button" id="more-actions" aria-label="${tr("その他のメニュー", "More options")}" aria-haspopup="menu" aria-expanded="${moreMenuOpen}">…</button>${moreMenuOpen ? `<div class="overflow-menu" role="menu" aria-label="${tr("その他のメニュー", "More options")}">
    <button type="button" role="menuitem" data-menu-action="shortcuts"><span aria-hidden="true">⌨</span><span>${tr("操作方法", "Controls")}</span></button>
    <button type="button" role="menuitem" data-menu-action="diagnostics"><span aria-hidden="true">✓</span><span>${tr("セットアップ診断", "Setup diagnostics")}</span></button>
    <button type="button" role="menuitem" data-menu-action="log"><span aria-hidden="true">≡</span><span>${tr("アプリログ", "Application log")}</span></button>
    <button type="button" role="menuitem" data-menu-action="copy-diagnostics"><span aria-hidden="true">⧉</span><span>${tr("匿名の診断概要をコピー", "Copy anonymous summary")}</span></button>
    <div class="overflow-separator" role="separator"></div>
    <button type="button" role="menuitem" data-menu-action="update" ${updateStatus === "checking" ? "disabled" : ""}><span aria-hidden="true">↻</span><span>${escapeHtml(updateLabel)}<small role="status">${escapeHtml(updateStatusText())}</small></span></button>
    ${updateStatus === "available" ? `<button type="button" role="menuitem" data-menu-action="release"><span aria-hidden="true">↗</span><span>${tr("ダウンロードページを開く", "Open download page")}</span></button>` : ""}
  </div>` : ""}</div>`;
}
function restoreFocus(activeId: string, modalCloseId?: string) {
  if (modalCloseId) {
    document.getElementById(modalCloseId)?.focus();
  } else if (activeId) {
    document.getElementById(activeId)?.focus();
  }
}
function bindDiagnosticsActions() {
  document.querySelector("#open-bluetooth-settings")?.addEventListener("click", () => invoke("open_bluetooth_settings"));
  document.querySelector("#open-sound-settings")?.addEventListener("click", () => invoke("open_sound_settings"));
  document.querySelector("#copy-diagnostics")?.addEventListener("click", copyDiagnosticSummary);
}
function bindDialogActions() {
  document.querySelector("#close-log")?.addEventListener("click", () => { logOpen = false; render(); });
  document.querySelector("#close-diagnostics")?.addEventListener("click", () => { diagnosticsOpen = false; render(); });
  document.querySelector("#close-shortcuts")?.addEventListener("click", () => { shortcutsOpen = false; render(); });
  document.querySelector("#refresh-log")?.addEventListener("click", showLog);
  document.querySelector("#copy-log")?.addEventListener("click", copyLog);
  document.querySelector("#open-log-folder")?.addEventListener("click", () => invoke("open_log_folder"));
  bindDiagnosticsActions();
}
function bindOverflowMenu() {
  document.querySelector("#more-actions")?.addEventListener("click", () => { moreMenuOpen = !moreMenuOpen; render(); });
  document.querySelectorAll<HTMLButtonElement>("[data-menu-action]").forEach((button) => button.addEventListener("click", () => {
    const action = button.dataset.menuAction;
    moreMenuOpen = false;
    if (action === "shortcuts") { shortcutsOpen = true; render(); }
    else if (action === "diagnostics") void showDiagnostics();
    else if (action === "log") void showLog();
    else if (action === "copy-diagnostics") { render(); void copyDiagnosticSummary(); }
    else if (action === "update") { moreMenuOpen = true; void checkForUpdates(); }
    else if (action === "release") { render(); void invoke("open_release_page"); }
  }));
  document.querySelector("#footer-update")?.addEventListener("click", () => invoke("open_release_page"));
}
function versionParts(value: string) {
  return value.replace(/^v/i, "").split(".").map((part) => Number.parseInt(part, 10) || 0);
}
function isNewerVersion(candidate: string, current: string) {
  const left = versionParts(candidate);
  const right = versionParts(current);
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference > 0;
  }
  return false;
}
function updateStatusText() {
  if (updateStatus === "checking") return tr("GitHubで最新版を確認しています…", "Checking GitHub for the latest version…");
  if (updateStatus === "available") return tr(`新しいバージョン ${latestVersion} があります。`, `Version ${latestVersion} is available.`);
  if (updateStatus === "current") return tr("現在のバージョンは最新です。", "You are using the latest version.");
  if (updateStatus === "error") return tr("最新版を確認できませんでした。ネットワーク接続を確認してください。", "Could not check for updates. Check your network connection.");
  return tr("ボタンを押したときだけGitHubへ接続します。", "Connects to GitHub only when you select the button.");
}
async function checkForUpdates() {
  updateStatus = "checking";
  render();
  try {
    const response = await fetch("https://api.github.com/repos/mouiron/phone-audio-receiver/releases/latest", { cache: "no-store", headers: { Accept: "application/vnd.github+json" } });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const release = await response.json() as { tag_name?: string };
    latestVersion = (release.tag_name ?? "").replace(/^v/i, "");
    if (!latestVersion) throw new Error("Release version is missing");
    updateStatus = isNewerVersion(latestVersion, appVersion) ? "available" : "current";
  } catch {
    updateStatus = "error";
  }
  render();
}
async function copyDiagnosticSummary() {
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const connecting = snapshot.devices.filter((device) => device.status === "connecting").length;
  const lines = [
    `Phone Audio Receiver ${appVersion || "unknown"}`,
    `Language: ${displayLanguage()}`,
    `Available devices: ${snapshot.devices.length}`,
    `Connected devices: ${connected}`,
    `Connecting devices: ${connecting}`,
    `Startup auto-connect: ${snapshot.settings.autoConnect ? "enabled" : "disabled"}`,
    `Startup targets: ${snapshot.autoConnectDeviceIds.length}`,
  ];
  try {
    await navigator.clipboard.writeText(lines.join("\n"));
    setProgress(tr("端末名やIDを含まない診断概要をコピーしました。", "Copied a diagnostic summary without device names or IDs."));
  } catch (error) {
    setProgress(tr(`診断概要をコピーできませんでした: ${error}`, `Could not copy the diagnostic summary: ${error}`));
  }
}
function renderQuick() {
  const activeId = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
  applyTheme();
  document.body.classList.add("quick-mode");
  document.body.classList.toggle("modal-open", logOpen || diagnosticsOpen || shortcutsOpen);
  document.documentElement.classList.toggle("modal-open", logOpen || diagnosticsOpen || shortcutsOpen);
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const query = deviceSearch.trim().toLocaleLowerCase();
  const devices = sortedDevices().filter((device) => {
    const matchesFilter = deviceFilter === "all" || (deviceFilter === "connected" ? device.status !== "disconnected" : device.status === "disconnected");
    return matchesFilter && (!query || device.name.toLocaleLowerCase().includes(query) || device.systemName.toLocaleLowerCase().includes(query));
  });
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `<main class="quick-shell">
    <header class="quick-header"><div class="brand"><img src="/icon.svg" aria-hidden="true"/><div><strong>Phone Audio Receiver</strong><span>${tr("クイック操作", "Quick controls")}</span></div></div><div class="quick-header-actions">${overflowMenu()}<button class="ghost quick-close" id="close-quick" aria-label="${tr("閉じる", "Close")}">×</button></div></header>
    <section class="quick-summary"><div><strong>${connected}</strong><span>${tr("台接続中", "connected")}</span></div><button class="app-action refresh-action compact-action" id="quick-refresh"><span>${tr("再検索", "Refresh")}</span></button></section>
    <div class="quick-tools"><input id="device-search" type="search" value="${escapeHtml(deviceSearch)}" placeholder="${tr("端末名を検索", "Search devices")}" aria-label="${tr("端末名を検索", "Search devices")}"/><div class="filter-group" role="group" aria-label="${tr("端末の絞り込み", "Filter devices")}">${(["all", "connected", "disconnected"] as DeviceFilter[]).map((filter) => `<button class="filter-button ${deviceFilter === filter ? "active" : ""}" data-filter="${filter}">${filter === "all" ? tr("すべて", "All") : filter === "connected" ? tr("接続中", "Connected") : tr("未接続", "Disconnected")}</button>`).join("")}</div></div>
    <section class="quick-devices" role="list">${devices.length ? devices.map((device) => `<article class="device quick-device ${device.status}" role="listitem" aria-label="${escapeHtml(`${device.name}: ${statusText(device.status)}`)}"><span class="dot ${device.status}" aria-hidden="true"></span><div class="device-name">${deviceNameMarkup(device)}<span>${statusText(device.status)}${device.favorite ? ` · ${tr("お気に入り", "Favorite")}` : ""}${device.autoConnect ? ` · ${tr("自動接続", "Auto-connect")}` : ""}</span>${deviceMessages.has(device.id) ? `<small>${escapeHtml(deviceMessages.get(device.id)!)}</small>` : ""}</div>${disconnectTimerControl(device)}<button class="${device.status === "disconnected" ? "primary" : "secondary"}" data-device="${encodeURIComponent(device.id)}" data-action="${device.status === "disconnected" ? "connect" : "disconnect"}">${device.status === "disconnected" ? tr("接続", "Connect") : device.status === "connecting" ? tr("中止", "Cancel") : tr("切断", "Disconnect")}</button></article>`).join("") : `<div class="empty quick-empty"><strong>${tr("該当する端末がありません", "No matching devices")}</strong><span>${tr("検索条件を変更するか、再検索してください。", "Change the filter or refresh the device list.")}</span></div>`}</section>
    <footer class="quick-footer"><span>${tr(`全${snapshot.devices.length}台`, `${snapshot.devices.length} total`)}</span><button class="secondary" id="open-main-window">${tr("詳細画面を開く", "Open details")}</button></footer>
    ${logOpen ? logDialog() : ""}
    ${diagnosticsOpen ? diagnosticsDialog() : ""}
    ${shortcutsOpen ? shortcutsDialog() : ""}
  </main>`;
  restoreFocus(activeId, shortcutsOpen ? "close-shortcuts" : logOpen ? "close-log" : diagnosticsOpen ? "close-diagnostics" : undefined);
  document.querySelector("#close-quick")!.addEventListener("click", () => invoke("hide_quick_window"));
  document.querySelector("#quick-refresh")!.addEventListener("click", refresh);
  document.querySelector("#open-main-window")!.addEventListener("click", () => invoke("switch_to_main_window"));
  bindOverflowMenu();
  bindDialogActions();
  document.querySelector<HTMLInputElement>("#device-search")!.addEventListener("input", (event) => { deviceSearch = (event.target as HTMLInputElement).value; render(); });
  document.querySelectorAll<HTMLButtonElement>("[data-filter]").forEach((button) => button.addEventListener("click", () => { deviceFilter = button.dataset.filter as DeviceFilter; render(); }));
  bindDeviceActions();
  bindDisconnectTimers();
  bindCustomSelects();
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
  document.body.classList.toggle("modal-open", logOpen || diagnosticsOpen || shortcutsOpen);
  document.documentElement.classList.toggle("modal-open", logOpen || diagnosticsOpen || shortcutsOpen);
  const connected = snapshot.devices.filter((device) => device.status === "connected").length;
  const mainDevices = sortedDevices();
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
    <main class="shell">
      <header class="topbar">
        <div class="brand"><img src="/icon.svg" aria-hidden="true"/><span>Phone Audio Receiver</span></div>
        <div class="actions"><div class="language-toggle" role="group" aria-label="${tr("表示言語", "Display language")}"><button type="button" data-language="ja" class="${displayLanguage() === "ja" ? "active" : ""}" aria-pressed="${displayLanguage() === "ja"}">日本語</button><button type="button" data-language="en" class="${displayLanguage() === "en" ? "active" : ""}" aria-pressed="${displayLanguage() === "en"}">EN</button></div><button class="ghost" id="open-quick-window">${tr("クイック表示", "Quick view")}</button>${overflowMenu()}</div>
      </header>
      <section class="panel devices-panel">
        <div class="panel-heading"><div><div class="devices-title"><h2>${tr("接続端末", "Devices")}</h2><span class="connection-count" aria-live="polite"><strong>${connected}</strong>${tr("台接続中", "connected")}</span></div><p id="connection-progress" role="status" aria-live="polite">${escapeHtml(progress)}</p></div><button class="app-action refresh-action refresh-button" id="refresh"><span>${tr("再検索", "Refresh")}</span></button></div>
        <div class="devices" role="list">${mainDevices.length ? mainDevices.map((device, index) => `
          <article class="device main-device ${device.status}" role="listitem" aria-label="${escapeHtml(`${device.name}: ${statusText(device.status)}`)}">
            <div class="device-identity"><div class="device-icon" aria-hidden="true">${uiIcon("audio")}</div><div class="device-name">${deviceNameMarkup(device)}<span class="device-status"><span class="dot ${device.status}" aria-hidden="true"></span>${statusText(device.status)}</span></div></div>
            <div class="device-preferences" role="group" aria-label="${escapeHtml(tr(`${device.name} の表示設定`, `Display settings for ${device.name}`))}">
              <button class="device-preference ${device.favorite ? "active" : ""}" data-favorite="${encodeURIComponent(device.id)}" aria-pressed="${device.favorite}" aria-label="${escapeHtml(tr(`${device.name} のお気に入りを切り替え`, `Toggle favorite for ${device.name}`))}" title="${tr("お気に入り", "Favorite")}">${uiIcon("favorite")}</button>
              <button class="device-preference ${device.trayPinned ? "active" : ""}" data-tray-pinned="${encodeURIComponent(device.id)}" aria-pressed="${device.trayPinned}" aria-label="${escapeHtml(tr(`${device.name} のタスクトレイ固定を切り替え`, `Toggle system tray pin for ${device.name}`))}" title="${tr("タスクトレイへ固定", "Pin to system tray")}">${uiIcon("pin")}</button>
              <button class="device-preference" data-edit-alias="${encodeURIComponent(device.id)}" title="${tr("表示名を変更", "Rename")}" aria-label="${escapeHtml(tr(`${device.name} の表示名を変更`, `Rename ${device.name}`))}">${uiIcon("edit")}</button>
              <span class="preference-separator" aria-hidden="true"></span>
              <button class="device-preference" data-move-device="${encodeURIComponent(device.id)}" data-direction="up" title="${tr("上へ移動", "Move up")}" aria-label="${escapeHtml(tr(`${device.name} を上へ移動`, `Move ${device.name} up`))}" ${index === 0 ? "disabled" : ""}>${uiIcon("up")}</button>
              <button class="device-preference" data-move-device="${encodeURIComponent(device.id)}" data-direction="down" title="${tr("下へ移動", "Move down")}" aria-label="${escapeHtml(tr(`${device.name} を下へ移動`, `Move ${device.name} down`))}" ${index === mainDevices.length - 1 ? "disabled" : ""}>${uiIcon("down")}</button>
            </div>
            <div class="device-connection-controls">
              <button class="device-auto-connect ${device.autoConnect ? "active" : ""}" data-auto-connect="${encodeURIComponent(device.id)}" aria-pressed="${device.autoConnect}" ${snapshot.settings.autoConnect ? "" : "disabled"} title="${snapshot.settings.autoConnect ? tr("この端末を起動時の自動接続対象にする", "Automatically connect to this device at startup") : tr("設定で「起動時に選択した端末へ自動接続する」をオンにすると選択できます", "Enable startup auto-connect in Settings before selecting devices")}"><span class="toggle-track" aria-hidden="true"><span></span></span><span>${tr("起動時に自動接続", "Auto-connect at startup")}</span></button>
              <div class="device-timer-field"><span>${tr("自動切断", "Auto disconnect")}</span>${disconnectTimerControl(device)}</div>
              ${mainConnectionButton(device)}
            </div>
          </article>`).join("") : `<div class="empty"><strong>${tr("まだ端末がありません", "No devices found yet")}</strong><span>${tr("「再検索」を押すと、音声受信に利用できるペア済みのBluetooth端末を表示します。", "Select Refresh to show paired Bluetooth devices available for audio reception.")}</span></div>`}</div>
      </section>
      <section class="panel settings"><div class="panel-heading"><div><h2>${tr("設定", "Settings")}</h2><p>${tr("起動時の動作と表示を調整できます。", "Configure startup behavior and appearance.")}</p></div></div>
        <div class="settings-grid">
          ${checkbox("autoConnect", tr("起動時に選択した端末へ自動接続する", "Connect to selected devices at startup"), snapshot.settings.autoConnect)}
          ${checkbox("connectionNotifications", tr("接続状態をWindows通知で知らせる", "Show Windows notifications for connection changes"), snapshot.settings.connectionNotifications)}
          ${checkbox("launchAtLogin", tr("Windows 起動時にアプリを起動する", "Launch the app when signing in to Windows"), snapshot.settings.launchAtLogin)}
          ${checkbox("startMinimized", tr("Windows 起動時は最小化して開始する", "Start minimized when launched with Windows"), snapshot.settings.startMinimized)}
          ${checkbox("minimizeToTray", tr("閉じるときにタスクトレイへ格納する", "Minimize to the system tray when closing"), snapshot.settings.minimizeToTray)}
          <div class="setting-option theme-control"><span class="setting-label">${tr("テーマ", "Theme")}</span>${themeControl()}</div>
        </div>
      </section>
      <footer class="app-footer"><span class="footer-product">Phone Audio Receiver${appVersion ? ` <span>v${escapeHtml(appVersion)}</span>` : ""}</span>${updateStatus === "available" ? `<button class="footer-update" id="footer-update">${escapeHtml(tr(`v${latestVersion} が利用可能`, `v${latestVersion} available`))} <span aria-hidden="true">→</span></button>` : ""}</footer>
      ${logOpen ? logDialog() : ""}
      ${diagnosticsOpen ? diagnosticsDialog() : ""}
      ${shortcutsOpen ? shortcutsDialog() : ""}
    </main>`;
  if (previousLogScroll !== undefined) document.querySelector<HTMLElement>(".log-content")?.scrollTo({ top: previousLogScroll });
  restoreFocus(activeId, shortcutsOpen ? "close-shortcuts" : logOpen ? "close-log" : diagnosticsOpen ? "close-diagnostics" : undefined);
  document.querySelector("#refresh")!.addEventListener("click", refresh);
  document.querySelectorAll<HTMLButtonElement>("[data-language]").forEach((button) => button.addEventListener("click", () => setLanguage(button.dataset.language as Snapshot["displayLanguage"])));
  document.querySelector("#open-quick-window")!.addEventListener("click", () => invoke("switch_to_quick_window"));
  bindOverflowMenu();
  bindDialogActions();
  bindDeviceActions();
  bindDisconnectTimers();
  bindCustomSelects();
  bindDevicePreferences();
  document.querySelectorAll<HTMLButtonElement>("[data-auto-connect]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.autoConnect!);
    try {
      await invoke("set_device_auto_connect", { deviceId: id, enabled: !button.classList.contains("active") });
      setProgress(tr("端末ごとの自動接続設定を保存しました。", "Device auto-connect setting saved."));
    } catch (error) { setProgress(tr(`設定を保存できませんでした: ${error}`, `Could not save the setting: ${error}`)); }
  }));
  document.querySelectorAll<HTMLInputElement>("input[data-setting]").forEach((input) => input.addEventListener("change", () => { void saveSettings(); }));
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
function bindDisconnectTimers() {
  document.querySelectorAll<HTMLSelectElement>("[data-disconnect-timer]").forEach((select) => select.addEventListener("change", async () => {
    const id = decodeURIComponent(select.dataset.disconnectTimer!);
    const minutes = Number(select.value);
    await setDisconnectTimer(id, minutes);
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-timer-option]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.timerDevice!);
    const minutes = Number(button.dataset.timerOption);
    openSelectId = null;
    render();
    focusCustomSelectTrigger(`timer:${id}`);
    await setDisconnectTimer(id, minutes);
  }));
}
async function setDisconnectTimer(id: string, minutes: number) {
  try {
    await invoke("set_disconnect_timer", { deviceId: id, minutes });
    setProgress(minutes === 0 ? tr("切断タイマーを解除しました。", "Disconnect timer cancelled.") : tr(`${minutes}分後に切断します。`, `The device will disconnect in ${minutes} minutes.`));
  } catch (error) {
    deviceMessages.set(id, String(error));
    setProgress(tr(`切断タイマーを設定できませんでした: ${error}`, `Could not set the disconnect timer: ${error}`));
    render();
  }
}
function bindCustomSelects() {
  document.querySelectorAll<HTMLButtonElement>("[data-select-toggle]").forEach((button) => button.addEventListener("click", () => {
    const selectId = decodeURIComponent(button.dataset.selectToggle!);
    const opening = openSelectId !== selectId;
    openSelectId = opening ? selectId : null;
    render();
    if (opening) {
      const selected = document.querySelector<HTMLButtonElement>('.custom-select.open [role="option"][aria-selected="true"]');
      (selected ?? document.querySelector<HTMLButtonElement>('.custom-select.open [role="option"]'))?.focus();
    }
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-theme-option]").forEach((button) => button.addEventListener("click", () => {
    openSelectId = null;
    snapshot.settings.theme = button.dataset.themeOption as Settings["theme"];
    render();
    document.querySelector<HTMLButtonElement>("#theme")?.focus();
    void saveSettings();
  }));
}
function focusCustomSelectTrigger(selectId: string) {
  const triggerId = selectId === "theme" ? "theme" : `timer-trigger-${encodeURIComponent(selectId.slice("timer:".length))}`;
  document.getElementById(triggerId)?.focus();
}
function bindDevicePreferences() {
  document.querySelectorAll<HTMLButtonElement>("[data-edit-alias]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.editAlias!);
    const device = snapshot.devices.find((candidate) => candidate.id === id);
    if (!device) return;
    const currentAlias = device.name === device.systemName ? "" : device.name;
    const alias = window.prompt(tr("アプリ内で使用する表示名を入力してください。空欄にすると元の端末名へ戻ります。", "Enter the name used in this app. Leave it blank to restore the original device name."), currentAlias);
    if (alias === null) return;
    try {
      await invoke("set_device_alias", { deviceId: id, alias });
      setProgress(tr("端末の表示名を保存しました。", "Device name saved."));
    } catch (error) { setProgress(tr(`表示名を保存できませんでした: ${error}`, `Could not save the device name: ${error}`)); }
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-favorite]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.favorite!);
    try { await invoke("set_device_favorite", { deviceId: id, enabled: button.getAttribute("aria-pressed") !== "true" }); }
    catch (error) { setProgress(tr(`お気に入りを変更できませんでした: ${error}`, `Could not update the favorite: ${error}`)); }
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-tray-pinned]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.trayPinned!);
    try { await invoke("set_device_tray_pinned", { deviceId: id, enabled: button.getAttribute("aria-pressed") !== "true" }); }
    catch (error) { setProgress(tr(`トレイ固定を変更できませんでした: ${error}`, `Could not update the tray pin: ${error}`)); }
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-move-device]").forEach((button) => button.addEventListener("click", async () => {
    const id = decodeURIComponent(button.dataset.moveDevice!);
    try {
      await invoke("move_device", { deviceId: id, direction: button.dataset.direction });
      setProgress(tr("端末の表示順を変更しました。", "Device order updated."));
    }
    catch (error) { setProgress(tr(`表示順を変更できませんでした: ${error}`, `Could not change the device order: ${error}`)); }
  }));
}
function handleKeyboard(event: KeyboardEvent) {
  if (openSelectId && ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    const items = [...document.querySelectorAll<HTMLButtonElement>('.custom-select.open [role="option"]')];
    if (!items.length) return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const target = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowDown" ? (current + 1) % items.length : (current <= 0 ? items.length : current) - 1;
    items[target]?.focus();
    return;
  }
  if (moreMenuOpen && ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    const items = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')];
    if (!items.length) return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const target = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowDown" ? (current + 1) % items.length : (current <= 0 ? items.length : current) - 1;
    items[target]?.focus();
    return;
  }
  if (event.key === "F5" && !event.ctrlKey && !event.altKey && !event.shiftKey) {
    event.preventDefault();
    void refresh();
    return;
  }
  if (event.key.toLowerCase() === "q" && event.altKey && !event.ctrlKey) {
    event.preventDefault();
    void invoke(quickMode ? "switch_to_main_window" : "switch_to_quick_window");
    return;
  }
  if (quickMode && event.key.toLowerCase() === "f" && event.ctrlKey) {
    event.preventDefault();
    document.querySelector<HTMLInputElement>("#device-search")?.focus();
    return;
  }
  if (event.key !== "Escape") return;
  if (openSelectId) {
    const selectId = openSelectId;
    openSelectId = null;
    render();
    focusCustomSelectTrigger(selectId);
  } else if (moreMenuOpen) {
    moreMenuOpen = false;
    render();
  } else if (logOpen) {
    logOpen = false;
    render();
  } else if (diagnosticsOpen) {
    diagnosticsOpen = false;
    render();
  } else if (shortcutsOpen) {
    shortcutsOpen = false;
    render();
  } else if (quickMode) {
    void invoke("hide_quick_window");
  }
}
function checkbox(key: keyof Settings, label: string, checked: boolean, disabled = false) { return `<label class="check setting-option"><span class="setting-label">${label}</span><input type="checkbox" data-setting="${key}" ${checked ? "checked" : ""} ${disabled ? "disabled" : ""}/><span class="setting-switch" aria-hidden="true"><span></span></span></label>`; }
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
async function setLanguage(language: Snapshot["displayLanguage"]) {
  if (language === displayLanguage()) return;
  const previousLanguage = displayLanguage();
  languageOverride = language;
  progress = tr("表示言語を日本語に切り替えました。", "Display language changed to English.");
  render();
  try {
    await invoke("set_display_language", { language });
  } catch (error) {
    languageOverride = previousLanguage;
    progress = tr(`表示言語を保存できませんでした: ${error}`, `Could not save the display language: ${error}`);
    render();
  }
}
async function refresh() { setProgress(tr("Bluetooth デバイスを検索しています…", "Searching for Bluetooth devices…")); try { snapshot = await invoke<Snapshot>("refresh_devices"); progress = snapshot.devices.length ? tr(`${snapshot.devices.length} 台の端末を見つけました。`, `Found ${snapshot.devices.length} device${snapshot.devices.length === 1 ? "" : "s"}.`) : tr("利用できるペア済み端末は見つかりませんでした。", "No paired devices are currently available."); } catch (error) { progress = tr(`検索できませんでした: ${error}`, `Could not search for devices: ${error}`); } render(); }
async function saveSettings() {
  const setting = (key: keyof Settings) => document.querySelector<HTMLInputElement>(`[data-setting="${key}"]`)?.checked ?? snapshot.settings[key];
  const notificationsEnabled = setting("connectionNotifications") as boolean;
  snapshot.settings = { ...snapshot.settings, autoConnect: setting("autoConnect") as boolean, connectionNotifications: notificationsEnabled, launchAtLogin: setting("launchAtLogin") as boolean, startMinimized: setting("startMinimized") as boolean, minimizeToTray: setting("minimizeToTray") as boolean };
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
document.addEventListener("keydown", handleKeyboard);
document.addEventListener("click", (event) => {
  if (moreMenuOpen && event.target instanceof Element && !event.target.closest(".overflow-actions")) {
    moreMenuOpen = false;
    render();
  }
  if (openSelectId && event.target instanceof Element && !event.target.closest(".custom-select")) {
    openSelectId = null;
    render();
  }
});
window.setInterval(updateTimerCountdowns, 1000);
void start();

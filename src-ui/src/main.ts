import "./style.css";
import {
  applyUpdates,
  cancelUpdates,
  checkUpdates,
  DEFAULT_SETTINGS,
  isTauri,
  loadSettings,
  saveSettings,
} from "./api";
import {
  formatBytes,
  sourceLabel,
  type ApplyOutcome,
  type Settings,
  type UpdateItem,
  type UpdatePlan,
  type UpdateSource,
} from "./types";

type ActiveTab = "system" | "flatpak";

interface State {
  plan: UpdatePlan | null;
  active: ActiveTab;
  selected: Set<UpdateSource>;
  detail: string | null;
  settings: Settings;
  loading: boolean;
  error: string | null;
  notice: string | null;
  settingsOpen: boolean;
  applying: boolean;
  cancelRequested: boolean;
  progress: number;
  progressMessage: string;
  log: string[];
}

const state: State = {
  plan: null,
  active: "system",
  selected: new Set(["system", "flatpak-system", "flatpak-user"]),
  detail: null,
  settings: DEFAULT_SETTINGS,
  loading: false,
  error: null,
  notice: null,
  settingsOpen: false,
  applying: false,
  cancelRequested: false,
  progress: 0,
  progressMessage: "Preparing transaction…",
  log: [],
};

const appElement = document.querySelector<HTMLDivElement>("#app");
if (!appElement) throw new Error("missing #app");
const app: HTMLDivElement = appElement;

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function selectedItems(): UpdateItem[] {
  if (!state.plan) return [];
  return state.plan.items.filter((item) => state.selected.has(item.source));
}

function itemsFor(source: UpdateSource): UpdateItem[] {
  if (!state.plan) return [];
  return state.plan.items.filter((item) => item.source === source);
}

function itemsForTab(tab: ActiveTab): UpdateItem[] {
  if (!state.plan) return [];
  if (tab === "system") {
    return state.plan.items.filter((item) => item.source === "system");
  }
  return state.plan.items.filter(
    (item) => item.source === "flatpak-system" || item.source === "flatpak-user",
  );
}

function flatpakItems(source: "flatpak-system" | "flatpak-user"): UpdateItem[] {
  return itemsFor(source);
}

function tabCount(tab: ActiveTab): number {
  return itemsForTab(tab).length;
}

function sourceCount(source: UpdateSource): number {
  return itemsFor(source).length;
}

function currentDetail(): UpdateItem | undefined {
  const items = itemsForTab(state.active);
  return items.find((item) => item.id === state.detail) ?? items[0];
}

function applySettingsToSelection(settings: Settings): void {
  if (settings.include_flatpak_system) state.selected.add("flatpak-system");
  else state.selected.delete("flatpak-system");
  if (settings.include_flatpak_user) state.selected.add("flatpak-user");
  else state.selected.delete("flatpak-user");
}

function rowHtml(item: UpdateItem, detail: UpdateItem | undefined): string {
  return `
    <button class="row ${detail?.id === item.id ? "selected" : ""}" data-detail="${escapeHtml(item.id)}">
      <span class="pkg">
        <span class="badge">${escapeHtml(item.repo)}</span>
        <span class="pkg-name">${escapeHtml(item.name)}</span>
        <span class="pkg-summary">${escapeHtml(item.summary)}</span>
      </span>
      <span class="ver"><span>${escapeHtml(item.current_version)}</span><span class="arrow">→</span><span class="new">${escapeHtml(item.new_version)}</span></span>
      <span class="size">${formatBytes(item.download_bytes)}</span>
    </button>`;
}

function flatpakGroup(
  source: "flatpak-system" | "flatpak-user",
  label: string,
  detail: UpdateItem | undefined,
): string {
  const items = flatpakItems(source);
  if (items.length === 0) return "";
  return `
    <div class="group-head">${label} <span class="count-pill">${items.length}</span></div>
    ${items.map((item) => rowHtml(item, detail)).join("")}`;
}

let renderQueued = false;
let autoRefreshTimer: number | null = null;

function scheduleAutoRefresh(): void {
  if (autoRefreshTimer !== null) {
    window.clearInterval(autoRefreshTimer);
    autoRefreshTimer = null;
  }
  if (!isTauri) return;
  const hours = state.settings.check_interval_hours;
  if (hours <= 0) return;
  autoRefreshTimer = window.setInterval(
    () => {
      if (!state.applying && !state.loading) void refresh();
    },
    hours * 60 * 60 * 1000,
  );
}

function scheduleRender(): void {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    render();
  });
}

function logLineClass(line: string): string {
  const lower = line.toLowerCase();
  if (lower.includes("error:") || lower.includes("failed")) return "err";
  if (lower.includes("warning:")) return "warn";
  if (line.startsWith("::")) return "ok";
  return "";
}

function progressView(): string {
  const lines = state.log.slice(-200);
  return `
    <section class="panel progress-panel">
      <div class="progress-head">
        <div>
          <h2>Updating the system</h2>
          <p>Do not power off the system during the update.</p>
        </div>
        <span class="chip progress-chip">${state.progress}%</span>
      </div>
      <div class="bar"><i style="width:${state.progress}%"></i></div>
      <div class="progress-meta">
        <span>${escapeHtml(state.progressMessage || "Preparing transaction…")}</span>
        <span>${state.progress}%</span>
      </div>
      <div class="log" id="update-log">${lines
        .map((line) => `<span class="${logLineClass(line)}">${escapeHtml(line)}</span>`)
        .join("\n")}</div>
      <div class="progress-actions">
        <button class="btn ghost" id="cancel" ${state.cancelRequested ? "disabled" : ""}>${state.cancelRequested ? "Cancelling…" : "Cancel"}</button>
      </div>
    </section>`;
}

function render(): void {
  const selected = selectedItems();
  const total = selected.reduce((sum, item) => sum + item.download_bytes, 0);
  const reboot = selected.some((item) => item.reboot_suggested);
  const detail = currentDetail();

  app.innerHTML = `
    <div class="app">
      <header class="header">
        <div>
          <h1>Skemer</h1>
          <p>${state.plan ? `Last checked <b>${escapeHtml(state.plan.checked_at)}</b>` : "Veldmuis updater"}</p>
        </div>
        <div class="header-actions">
          <button class="btn ghost" id="settings" title="Settings" ${state.applying ? "disabled" : ""}>Settings</button>
          <button class="btn" id="check" ${state.loading || state.applying ? "disabled" : ""}>${state.loading ? "Checking…" : "Check for updates"}</button>
        </div>
      </header>

      ${state.error ? `<div class="banner error">${escapeHtml(state.error)}</div>` : ""}
      ${state.notice ? `<div class="banner success">${escapeHtml(state.notice)}</div>` : ""}

      <section class="summary ${state.applying ? "hidden" : ""}">
        <div class="count">${selected.length}</div>
        <div class="summary-text">
          <strong>${selected.length === 0 ? "No selected updates" : "updates selected"}</strong>
          <span>${formatBytes(total)} to download</span>
        </div>
        <div class="summary-actions">
          ${reboot ? `<span class="chip reboot">Reboot suggested</span>` : ""}
          <button class="btn primary" id="update" ${state.loading || selected.length === 0 ? "disabled" : ""}>Update all</button>
        </div>
      </section>

      <nav class="tabs ${state.applying ? "hidden" : ""}">
        <button class="tab ${state.active === "system" ? "active" : ""}" data-tab="system">
          System <span class="count-pill">${tabCount("system")}</span>
        </button>
        <button class="tab ${state.active === "flatpak" ? "active" : ""}" data-tab="flatpak">
          Flatpak <span class="count-pill">${tabCount("flatpak")}</span>
        </button>
      </nav>

      <section class="workspace ${state.applying ? "hidden" : ""}">
        <div class="panel list-panel">
          ${
            state.active === "system"
              ? `
                <div class="list-head">
                  <label class="source-toggle">
                    <input type="checkbox" data-toggle-source="system" ${state.selected.has("system") ? "checked" : ""}>
                    Include System
                  </label>
                  <span>Version</span>
                  <span class="right">Size</span>
                </div>
                <div class="list">
                  ${
                    itemsForTab("system").length === 0
                      ? `<div class="empty"><strong>Nothing to update</strong>The system is up to date.</div>`
                      : itemsForTab("system")
                          .map((item) => rowHtml(item, detail))
                          .join("")
                  }
                </div>`
              : `
                <div class="list-head flatpak-head">
                  <div class="source-toggles">
                    <label class="source-toggle">
                      <input type="checkbox" data-toggle-source="flatpak-system" ${state.selected.has("flatpak-system") ? "checked" : ""}> System
                    </label>
                    <label class="source-toggle">
                      <input type="checkbox" data-toggle-source="flatpak-user" ${state.selected.has("flatpak-user") ? "checked" : ""}> User
                    </label>
                  </div>
                  <span>Version</span>
                  <span class="right">Size</span>
                </div>
                <div class="list">
                  ${
                    itemsForTab("flatpak").length === 0
                      ? `<div class="empty"><strong>Nothing to update</strong>Flatpak is up to date.</div>`
                      : flatpakGroup("flatpak-system", "System", detail) +
                        flatpakGroup("flatpak-user", "User", detail)
                  }
                </div>`
          }
        </div>

        <aside class="panel details">
          ${
            detail
              ? `
                <div class="eyebrow">Selected package</div>
                <h2>${escapeHtml(detail.name)}</h2>
                <div class="repo"><span class="badge">${escapeHtml(detail.repo)}</span>${escapeHtml(detail.summary)}</div>
                <p class="desc">This update moves <b>${escapeHtml(detail.name)}</b> from <code>${escapeHtml(detail.current_version)}</code> to <code>${escapeHtml(detail.new_version)}</code>.</p>
                <dl class="kv">
                  <dt>Source</dt><dd>${escapeHtml(sourceLabel(detail.source))}</dd>
                  <dt>Download</dt><dd>${formatBytes(detail.download_bytes)}</dd>
                  <dt>Reboot</dt><dd>${detail.reboot_suggested ? "Suggested" : "No"}</dd>
                </dl>
                ${detail.reboot_suggested ? `<div class="warn">Reboot suggested after updating. The system keeps running until you reboot.</div>` : ""}
              `
              : `<div class="empty"><strong>No package selected</strong>Select a package to see details.</div>`
          }
        </aside>
      </section>

      ${state.applying ? progressView() : ""}

      <footer class="footer">
        <span class="muted">${isTauri ? "Skemer is running natively." : "Browser preview with sample data."}</span>
        <span class="spacer"></span>
        <span class="muted">${state.plan ? `${state.plan.items.length} total updates` : "No plan yet"}</span>
      </footer>

      ${
        state.settingsOpen && !state.applying
          ? `
            <section class="settings panel">
              <h2>Settings</h2>
              <label>Check for updates
                <select id="setting-interval">
                  ${[0, 1, 3, 6, 12, 24]
                    .map(
                      (hours) =>
                        `<option value="${hours}" ${state.settings.check_interval_hours === hours ? "selected" : ""}>${hours === 0 ? "Never" : `Every ${hours} hour${hours === 1 ? "" : "s"}`}</option>`,
                    )
                    .join("")}
                </select>
              </label>
              <label><input type="checkbox" id="setting-notifications" ${state.settings.notifications ? "checked" : ""}> Desktop notifications</label>
              <label><input type="checkbox" id="setting-autostart" ${state.settings.autostart ? "checked" : ""}> Start at login</label>
              <label><input type="checkbox" id="setting-close-to-tray" ${state.settings.close_to_tray ? "checked" : ""}> Close to tray</label>
              <label><input type="checkbox" id="setting-flatpak-system" ${state.settings.include_flatpak_system ? "checked" : ""}> Include Flatpak system</label>
              <label><input type="checkbox" id="setting-flatpak-user" ${state.settings.include_flatpak_user ? "checked" : ""}> Include Flatpak user</label>
              <div class="settings-actions">
                <button class="btn ghost" id="settings-cancel">Cancel</button>
                <button class="btn primary" id="settings-save">Save</button>
              </div>
            </section>`
          : ""
      }
    </div>`;

  wire();

  if (state.applying) {
    const logElement = document.querySelector<HTMLDivElement>("#update-log");
    if (logElement) logElement.scrollTop = logElement.scrollHeight;
  }
}

function wire(): void {
  document.querySelector("#check")?.addEventListener("click", () => {
    state.notice = null;
    void refresh();
  });
  document.querySelector("#update")?.addEventListener("click", () => void apply());
  document.querySelector("#cancel")?.addEventListener("click", () => void cancel());
  document.querySelector("#settings")?.addEventListener("click", () => {
    state.settingsOpen = !state.settingsOpen;
    render();
  });
  document.querySelector("#settings-cancel")?.addEventListener("click", () => {
    state.settingsOpen = false;
    render();
  });
  document.querySelector("#settings-save")?.addEventListener("click", () => void saveSettingsFromForm());

  document.querySelectorAll<HTMLElement>("[data-tab]").forEach((element) => {
    element.addEventListener("click", () => {
      state.active = element.dataset.tab as ActiveTab;
      state.detail = null;
      render();
    });
  });

  document.querySelectorAll<HTMLInputElement>("[data-toggle-source]").forEach((element) => {
    element.addEventListener("change", () => {
      const source = element.dataset.toggleSource as UpdateSource;
      if (element.checked) state.selected.add(source);
      else state.selected.delete(source);
      render();
    });
  });

  document.querySelectorAll<HTMLElement>("[data-detail]").forEach((element) => {
    element.addEventListener("click", () => {
      state.detail = element.dataset.detail ?? null;
      render();
    });
  });
}

async function refresh(): Promise<void> {
  state.loading = true;
  state.error = null;
  render();
  try {
    state.plan = await checkUpdates();
    state.detail = currentDetail()?.id ?? null;
  } catch (error) {
    state.error = String(error);
  } finally {
    state.loading = false;
    render();
  }
}

async function apply(): Promise<void> {
  const sources = [...state.selected].filter((source) => sourceCount(source) > 0);
  if (sources.length === 0) return;

  state.applying = true;
  state.cancelRequested = false;
  state.progress = 0;
  state.progressMessage = "Preparing transaction…";
  state.log = [];
  state.error = null;
  state.notice = null;
  render();

  try {
    const outcome: ApplyOutcome = await applyUpdates(sources, []);
    state.applying = false;
    if (outcome.cancelled) {
      state.notice = "Update cancelled.";
    } else if (!outcome.success) {
      state.error = outcome.error ?? "The update did not complete.";
    } else {
      state.notice = "Update complete.";
    }
  } catch (error) {
    state.applying = false;
    state.error = String(error);
  }

  render();
  if (!state.error) {
    await refresh();
  }
}

async function cancel(): Promise<void> {
  if (state.cancelRequested) return;
  state.cancelRequested = true;
  state.progressMessage = "Cancelling…";
  render();
  try {
    await cancelUpdates();
  } catch (error) {
    state.error = String(error);
    render();
  }
}

async function subscribeToUpdates(): Promise<void> {
  if (!isTauri) return;
  const { listen } = await import("@tauri-apps/api/event");
  await listen<{ percent: number; message: string }>("skemer://update-progress", (event) => {
    const percent = Number(event.payload.percent);
    const next = Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : 0;
    state.progress = Math.max(state.progress, next);
    state.progressMessage = event.payload.message;
    scheduleRender();
  });
  await listen<{ line: string }>("skemer://update-log", (event) => {
    state.log.push(event.payload.line);
    if (state.log.length > 400) {
      state.log.splice(0, state.log.length - 400);
    }
    scheduleRender();
  });
}

async function saveSettingsFromForm(): Promise<void> {
  const interval = document.querySelector<HTMLSelectElement>("#setting-interval");
  const notifications = document.querySelector<HTMLInputElement>("#setting-notifications");
  const autostart = document.querySelector<HTMLInputElement>("#setting-autostart");
  const closeToTray = document.querySelector<HTMLInputElement>("#setting-close-to-tray");
  const flatpakSystem = document.querySelector<HTMLInputElement>("#setting-flatpak-system");
  const flatpakUser = document.querySelector<HTMLInputElement>("#setting-flatpak-user");

  state.settings = {
    check_interval_hours: Number(interval?.value ?? 6),
    notifications: Boolean(notifications?.checked),
    autostart: Boolean(autostart?.checked),
    close_to_tray: Boolean(closeToTray?.checked),
    include_flatpak_system: Boolean(flatpakSystem?.checked),
    include_flatpak_user: Boolean(flatpakUser?.checked),
  };

  try {
    await saveSettings(state.settings);
    applySettingsToSelection(state.settings);
    scheduleAutoRefresh();
    state.settingsOpen = false;
    await refresh();
  } catch (error) {
    state.error = String(error);
  }
  render();
}

void (async () => {
  try {
    state.settings = await loadSettings();
  } catch (error) {
    state.error = String(error);
  }
  applySettingsToSelection(state.settings);
  scheduleAutoRefresh();
  try {
    await subscribeToUpdates();
  } catch (error) {
    state.error = String(error);
  }
  render();
  await refresh();
})();

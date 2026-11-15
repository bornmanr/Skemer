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

interface State {
  plan: UpdatePlan | null;
  active: UpdateSource;
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

function sourceCount(source: UpdateSource): number {
  return itemsFor(source).length;
}

function currentDetail(): UpdateItem | undefined {
  const items = state.plan?.items ?? [];
  return items.find((item) => item.id === state.detail) ?? itemsFor(state.active)[0];
}

let renderQueued = false;

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
        ${(["system", "flatpak-system", "flatpak-user"] as UpdateSource[])
          .map(
            (source) => `
              <button class="tab ${state.active === source ? "active" : ""}" data-source="${source}">
                ${sourceLabel(source)} <span class="count-pill">${sourceCount(source)}</span>
              </button>`,
          )
          .join("")}
      </nav>

      <section class="workspace ${state.applying ? "hidden" : ""}">
        <div class="panel list-panel">
          <div class="list-head">
            <label class="source-toggle">
              <input type="checkbox" data-toggle-source="${state.active}" ${state.selected.has(state.active) ? "checked" : ""}>
              Include ${sourceLabel(state.active)}
            </label>
            <span>Version</span>
            <span class="right">Size</span>
          </div>
          <div class="list">
            ${
              itemsFor(state.active).length === 0
                ? `<div class="empty"><strong>Nothing to update</strong>This source is up to date.</div>`
                : itemsFor(state.active)
                    .map(
                      (item) => `
                        <button class="row ${detail?.id === item.id ? "selected" : ""}" data-detail="${escapeHtml(item.id)}">
                          <span class="pkg">
                            <span class="badge">${escapeHtml(item.repo)}</span>
                            <span class="pkg-name">${escapeHtml(item.name)}</span>
                            <span class="pkg-summary">${escapeHtml(item.summary)}</span>
                          </span>
                          <span class="ver"><span>${escapeHtml(item.current_version)}</span><span class="arrow">→</span><span class="new">${escapeHtml(item.new_version)}</span></span>
                          <span class="size">${formatBytes(item.download_bytes)}</span>
                        </button>`,
                    )
                    .join("")
            }
          </div>
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

  document.querySelectorAll<HTMLElement>("[data-source]").forEach((element) => {
    element.addEventListener("click", () => {
      state.active = element.dataset.source as UpdateSource;
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
    state.progress = Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : 0;
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
    state.settingsOpen = false;
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
  try {
    await subscribeToUpdates();
  } catch (error) {
    state.error = String(error);
  }
  render();
  await refresh();
})();

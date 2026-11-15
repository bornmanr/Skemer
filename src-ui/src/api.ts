import type { Settings, UpdateItem } from "./types";

export const DEFAULT_SETTINGS: Settings = {
  check_interval_hours: 6,
  notifications: true,
  autostart: true,
  close_to_tray: true,
  include_flatpak_system: true,
  include_flatpak_user: true,
};

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function loadSettings(): Promise<Settings> {
  if (!isTauri) return DEFAULT_SETTINGS;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<Settings>("load_settings");
}

export async function saveSettings(settings: Settings): Promise<void> {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("save_settings", { settings });
}

export async function checkUpdates(): Promise<import("./types").UpdatePlan> {
  if (!isTauri) return demoPlan();
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<import("./types").UpdatePlan>("check_updates");
}

export async function applyUpdates(
  sources: string[],
  holds: string[],
): Promise<import("./types").ApplyOutcome> {
  if (!isTauri) {
    throw new Error("Apply is not implemented in the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<import("./types").ApplyOutcome>("apply_updates", {
    sources,
    holds,
  });
}

function demoPlan(): import("./types").UpdatePlan {
  const items = [
    ["linux", "core", "6.17.7.arch1-1", "6.17.8.arch1-1", 142, true, "Linux kernel and modules"],
    ["systemd", "core", "257.8-1", "257.9-1", 18, true, "System and service manager"],
    ["mesa", "extra", "25.2.4-1", "25.2.5-1", 38, true, "Open-source graphics stack"],
    ["firefox", "extra", "144.0.2-1", "145.0-1", 76, false, "Standalone web browser"],
    ["git", "extra", "2.56.0-1", "2.56.1-1", 8, false, "Distributed version control"],
    ["python", "extra", "3.13.9-1", "3.13.10-1", 12, false, "Python interpreter"],
    ["libxml2", "core", "2.15.1-1", "2.15.2-1", 1.2, false, "XML parsing library"],
    ["veldmuis-branding", "veldmuis-core", "20260306-23", "20260306-24", 2.4, false, "Veldmuis branding assets"],
  ].map(([name, repo, current, next, size, reboot, summary]) => ({
    id: `system:${name}`,
    source: "system" as const,
    name: String(name),
    repo: String(repo),
    current_version: String(current),
    new_version: String(next),
    download_bytes: Number(size) * 1024 * 1024,
    installed_bytes: 0,
    summary: String(summary),
    reboot_suggested: Boolean(reboot),
    warning: null,
  }));

  const flatpak = [
    ["org.gimp.GIMP", "3.0.2", "3.0.4", 64, "system"],
    ["org.freedesktop.Platform", "24.08", "24.08.2", 73, "system"],
    ["com.spotify.Client", "1.2.63", "1.2.64", 112, "user"],
    ["org.keepassxc.KeePassXC", "2.7.9", "2.7.10", 28, "user"],
  ].map(([name, current, next, size, scope]) => ({
    id: `flatpak:${scope}:${name}`,
    source: (scope === "system" ? "flatpak-system" : "flatpak-user") as UpdateItem["source"],
    name: String(name),
    repo: "flathub",
    current_version: String(current),
    new_version: String(next),
    download_bytes: Number(size) * 1024 * 1024,
    installed_bytes: 0,
    summary: "Flatpak application or runtime",
    reboot_suggested: false,
    warning: null,
  }));

  const all: UpdateItem[] = [...items, ...flatpak];
  return {
    items: all,
    total_download_bytes: all.reduce((sum, item) => sum + item.download_bytes, 0),
    reboot_suggested: all.some((item) => item.reboot_suggested),
    checked_at: "browser preview",
    warnings: [],
    held_packages: [],
  };
}

export type UpdateSource = "system" | "flatpak-system" | "flatpak-user";

export interface UpdateItem {
  id: string;
  source: UpdateSource;
  name: string;
  repo: string;
  current_version: string;
  new_version: string;
  download_bytes: number;
  installed_bytes: number;
  summary: string;
  reboot_suggested: boolean;
  warning: string | null;
}

export interface UpdatePlan {
  items: UpdateItem[];
  total_download_bytes: number;
  reboot_suggested: boolean;
  checked_at: string;
  warnings: string[];
  held_packages: string[];
}

export interface Settings {
  check_interval_hours: number;
  notifications: boolean;
  autostart: boolean;
  close_to_tray: boolean;
  include_flatpak_system: boolean;
  include_flatpak_user: boolean;
}

export interface ApplyOutcome {
  success: boolean;
  held_packages: string[];
  reboot_suggested: boolean;
  log_tail: string;
  error: string | null;
  candidates: string[];
}

export const SOURCE_LABELS: Record<UpdateSource, string> = {
  system: "System",
  "flatpak-system": "Flatpak system",
  "flatpak-user": "Flatpak user",
};

export function sourceLabel(source: UpdateSource): string {
  return SOURCE_LABELS[source] ?? source;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KiB`;
  return `${bytes} B`;
}

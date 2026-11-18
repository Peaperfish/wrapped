// Typed wrappers around the dashboard's Tauri commands (dashboard/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { ExportResult, GitEmail, Meta, SummaryJson, YearSeries } from "./types";

export const getMeta = (): Promise<Meta> => invoke<Meta>("meta");

/** `vs`: compare with this range instead of the previous range of equal length. */
export const getSummary = (
  from: string,
  to: string,
  vs?: { from: string; to: string },
): Promise<SummaryJson> =>
  invoke<SummaryJson>("summary", { from, to, vsFrom: vs?.from ?? null, vsTo: vs?.to ?? null });

export const getSeries = (year: number, metric: string): Promise<YearSeries> =>
  invoke<YearSeries>("series", { year, metric });

/** Display names for bundle ids; ids without an installed app come back unchanged. */
export const getAppNames = (ids: string[]): Promise<Record<string, string>> =>
  invoke<Record<string, string>>("app_names", { ids });

/** Ask a running collector to save now. False: none is running. */
export const flushCollector = (): Promise<boolean> =>
  invoke<boolean>("flush_collector").catch(() => false);

/** The first frame with data is on screen: the backend shows the window. */
export const ready = (): Promise<void> => invoke<void>("ready").catch(() => undefined);

/** The collector's answer to a request (see `ctl` in dashboard/src/lib.rs). */
export type CtlReply = { ok: true; data: any } | { ok: false; code: string; error: string };

/** A request the collector refused or could not answer. `code` is
 *  `not_running` when Wrapped is not running, `timeout` when it did not answer. */
export class CtlError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

/** Ask the running collector to do something (it is the only writer of the
 *  database). Resolves with its data, or throws a `CtlError`. */
export async function sendCtl(cmd: string, args?: unknown): Promise<any> {
  const r = await invoke<CtlReply>("ctl", { cmd, args: args ?? null });
  if (r.ok) return r.data;
  throw new CtlError(r.code, r.error);
}

/** True once when the tray's "Settings…" asked for Settings to open (the flag is consumed). */
export const takeOpenSettings = (): Promise<boolean> =>
  invoke<boolean>("take_open_settings").catch(() => false);

/** Open the Wrapped app (the collector), for "Wrapped isn't running". */
export const openWrapped = (): Promise<boolean> => invoke<boolean>("open_wrapped").catch(() => false);

/** Author emails in the repositories under these folders (read-only; can take seconds). */
export const gitEmails = (folders: string[]): Promise<GitEmail[]> =>
  invoke<GitEmail[]>("git_emails", { folders });

/** Bundle ids Wrapped has recorded, most time first. */
export const recordedApps = (): Promise<string[]> => invoke<string[]>("recorded_apps");

export const exportData = (kind: "json" | "csv", path: string): Promise<ExportResult> =>
  invoke<ExportResult>("export_data", { kind, path });

/** Native folder picker (several folders allowed). Empty when cancelled. */
export async function pickFolders(title: string, multiple = true): Promise<string[]> {
  const r = await openDialog({ directory: true, multiple, title });
  if (r == null) return [];
  return Array.isArray(r) ? r : [r];
}

/** Native "save as" dialog; null when cancelled. */
export async function pickSavePath(defaultPath: string): Promise<string | null> {
  return saveDialog({ defaultPath, filters: [{ name: "JSON", extensions: ["json"] }] });
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

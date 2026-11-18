// Typed wrappers around the dashboard's Tauri commands (dashboard/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import type { Meta, SummaryJson, YearSeries } from "./types";

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

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

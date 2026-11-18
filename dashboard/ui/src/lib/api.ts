// Typed wrappers around the dashboard's Tauri commands (dashboard/src/lib.rs).
import { invoke } from "@tauri-apps/api/core";
import type { Meta, SummaryJson, YearSeries } from "./types";

export const getMeta = (): Promise<Meta> => invoke<Meta>("meta");

export const getSummary = (from: string, to: string): Promise<SummaryJson> =>
  invoke<SummaryJson>("summary", { from, to });

export const getSeries = (year: number, metric: string): Promise<YearSeries> =>
  invoke<YearSeries>("series", { year, metric });

/** The first frame with data is on screen: the backend shows the window. */
export const ready = (): Promise<void> => invoke<void>("ready").catch(() => undefined);

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

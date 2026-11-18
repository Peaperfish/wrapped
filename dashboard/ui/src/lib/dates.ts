// Local calendar dates as "YYYY-MM-DD" strings (the same form the backend
// uses). Arithmetic runs on day numbers in UTC, so DST never shifts a day.
// Weeks start on Monday.

export type Ymd = string;
export interface Range {
  from: Ymd;
  to: Ymd;
}
export type Preset = "today" | "week" | "month" | "year" | "custom";

const DAY_MS = 86_400_000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const pad = (n: number): string => String(n).padStart(2, "0");

function parts(d: Ymd): [number, number, number] {
  const [y = "1970", m = "1", day = "1"] = d.split("-");
  return [Number(y), Number(m), Number(day)];
}

/** Days since 1970-01-01. */
export function dayNumber(d: Ymd): number {
  const [y, m, day] = parts(d);
  return Math.round(Date.UTC(y, m - 1, day) / DAY_MS);
}

export function fromDayNumber(n: number): Ymd {
  const t = new Date(n * DAY_MS);
  return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
}

export const addDays = (d: Ymd, n: number): Ymd => fromDayNumber(dayNumber(d) + n);
/** Inclusive number of days from `a` to `b`. */
export const daysIn = (r: Range): number => dayNumber(r.to) - dayNumber(r.from) + 1;
/** 0 = Monday … 6 = Sunday. */
export const weekday = (d: Ymd): number => (new Date(dayNumber(d) * DAY_MS).getUTCDay() + 6) % 7;
export const yearOf = (d: Ymd): number => parts(d)[0];
export const isValidYmd = (d: string): boolean =>
  /^\d{4}-\d{2}-\d{2}$/.test(d) && fromDayNumber(dayNumber(d)) === d;

/** `a` and `b` in order, as a range. */
export const ordered = (a: Ymd, b: Ymd): Range => (a <= b ? { from: a, to: b } : { from: b, to: a });
export const sameRange = (a: Range | null, b: Range | null): boolean =>
  !!a && !!b && a.from === b.from && a.to === b.to;

export function yearRange(y: number): Range {
  return { from: `${y}-01-01`, to: `${y}-12-31` };
}

export function presetRange(p: Exclude<Preset, "custom">, today: Ymd): Range {
  const [y, m] = parts(today);
  switch (p) {
    case "today":
      return { from: today, to: today };
    case "week": {
      const from = addDays(today, -weekday(today));
      return { from, to: addDays(from, 6) };
    }
    case "month": {
      const last = new Date(Date.UTC(y, m, 0)).getUTCDate();
      return { from: `${y}-${pad(m)}-01`, to: `${y}-${pad(m)}-${pad(last)}` };
    }
    case "year":
      return yearRange(y);
  }
}

const daysInMonth = (y: number, m: number): number => new Date(Date.UTC(y, m, 0)).getUTCDate();

/** What to ask the backend for a selection: the range to summarise and,
 *  for a preset period still in progress, the range to compare it with. */
export interface SummaryQuery extends Range {
  vs?: Range;
}

/**
 * A preset that contains today (this week / month / year) is summarised only
 * up to today and compared with the same elapsed days of the period before:
 * Mon..today vs last Mon..same weekday; the 1st..today vs the 1st..the same
 * day of the previous month (clamped to its length); Jan 1..today vs Jan 1..
 * the same date last year (Feb 29 becomes Feb 28). Everything else (today,
 * custom ranges, finished periods) has no explicit comparison: the backend
 * uses the previous range of equal length.
 */
export function summaryQuery(r: Range, today: Ymd): SummaryQuery {
  const p = presetOf(r, today);
  const [y, m, d] = parts(today);
  switch (p) {
    case "week": {
      const vsFrom = addDays(r.from, -7);
      return { from: r.from, to: today, vs: { from: vsFrom, to: addDays(vsFrom, weekday(today)) } };
    }
    case "month": {
      const [py, pm] = m === 1 ? [y - 1, 12] : [y, m - 1];
      const day = Math.min(d, daysInMonth(py, pm));
      return {
        from: r.from,
        to: today,
        vs: { from: `${py}-${pad(pm)}-01`, to: `${py}-${pad(pm)}-${pad(day)}` },
      };
    }
    case "year": {
      const day = m === 2 && d === 29 ? 28 : d;
      return {
        from: r.from,
        to: today,
        vs: { from: `${y - 1}-01-01`, to: `${y - 1}-${pad(m)}-${pad(day)}` },
      };
    }
    default:
      return { from: r.from, to: r.to };
  }
}

/** The preset a range is, or "custom". */
export function presetOf(r: Range, today: Ymd): Preset {
  for (const p of ["today", "week", "month", "year"] as const) {
    if (sameRange(r, presetRange(p, today))) return p;
  }
  return "custom";
}

/** "Tue 18 Nov 2026" (or without the year). */
export function fmtDay(d: Ymd, withYear = true): string {
  const [y, m, day] = parts(d);
  const s = `${WEEKDAYS[weekday(d)]} ${day} ${MONTHS[m - 1]}`;
  return withYear ? `${s} ${y}` : s;
}

export const monthShort = (m: number): string => MONTHS[m] ?? "";
export const weekdayShort = (i: number): string => WEEKDAYS[i] ?? "";

/** "18 Nov 2026", "3 – 9 Nov 2026", "November 2026", "2026", … */
export function rangeLabel(r: Range): string {
  const [y1, m1, d1] = parts(r.from);
  const [y2, m2, d2] = parts(r.to);
  if (r.from === r.to) return fmtDay(r.from);
  if (r.from === `${y1}-01-01` && r.to === `${y1}-12-31`) return String(y1);
  const lastOfMonth = new Date(Date.UTC(y2, m2, 0)).getUTCDate();
  if (y1 === y2 && m1 === m2 && d1 === 1 && d2 === lastOfMonth) return `${MONTHS_LONG[m1 - 1]} ${y1}`;
  if (y1 === y2 && m1 === m2) return `${d1} – ${d2} ${MONTHS[m2 - 1]} ${y2}`;
  if (y1 === y2) return `${d1} ${MONTHS[m1 - 1]} – ${d2} ${MONTHS[m2 - 1]} ${y2}`;
  return `${d1} ${MONTHS[m1 - 1]} ${y1} – ${d2} ${MONTHS[m2 - 1]} ${y2}`;
}

/** Local clock time of a UTC instant: "22:13", or "16 Nov 22:13" when not today. */
export function fmtAsOf(utcMs: number, today: Ymd): string {
  const t = new Date(utcMs);
  const day = `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}`;
  const clock = `${pad(t.getHours())}:${pad(t.getMinutes())}`;
  if (day === today) return clock;
  const sameYear = t.getFullYear() === yearOf(today);
  return `${t.getDate()} ${MONTHS[t.getMonth()]}${sameYear ? "" : ` ${t.getFullYear()}`} ${clock}`;
}

/** Today's local date in this process (the backend's `meta.today` wins once loaded). */
export function localToday(): Ymd {
  const t = new Date();
  return `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}`;
}

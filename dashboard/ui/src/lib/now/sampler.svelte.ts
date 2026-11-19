// The Now tab's sampler: polls `now_snapshot` once a second, but only while
// `setActive(true)` (the Now tab is shown AND the window is visible). One call
// at a time: the next is scheduled only after the previous one answered. When
// stopped, no timer is left, any answer still in flight is dropped, and the
// history is cleared so a gap never draws a line. Nothing is saved.
import { errorText, getNow } from "../api";
import type { NowSnapshotJson } from "../types";

export const HISTORY = 60;
const PERIOD_MS = 1000;

/** The history of one number per second; null where there was none. */
export type Series = (number | null)[];

export interface History {
  cpu: Series;
  gpu: Series;
  diskRead: Series;
  diskWrite: Series;
  netDown: Series;
  netUp: Series;
}

const empty = (): History => ({ cpu: [], gpu: [], diskRead: [], diskWrite: [], netDown: [], netUp: [] });

function push(s: Series, v: number | null) {
  s.push(v);
  if (s.length > HISTORY) s.shift();
}

class NowSampler {
  snap = $state.raw<NowSnapshotJson | null>(null);
  /** A new object every second (the arrays inside are mutated in place). */
  hist = $state.raw<History>(empty());
  error = $state<string | null>(null);

  #timer: ReturnType<typeof setTimeout> | undefined;
  #pending = false;
  #active = false;
  /** Bumped on every start / stop; answers of an older run are dropped. */
  #run = 0;
  #h = empty();

  setActive(on: boolean) {
    if (on === this.#active) return;
    this.#active = on;
    this.#run++;
    clearTimeout(this.#timer);
    this.#timer = undefined;
    if (on) {
      this.#pending = false;
      void this.#tick();
    } else {
      this.#pending = false;
      this.#h = empty();
      this.hist = this.#h;
      this.snap = null;
      this.error = null;
    }
  }

  async #tick() {
    if (this.#pending || !this.#active) return;
    this.#pending = true;
    const run = this.#run;
    const t0 = performance.now();
    try {
      const s = await getNow();
      if (run !== this.#run) return;
      const h = this.#h;
      push(h.cpu, s.cpu_percent);
      push(h.gpu, s.gpu_percent);
      push(h.diskRead, s.disk_read_per_sec);
      push(h.diskWrite, s.disk_write_per_sec);
      push(h.netDown, s.net_down_per_sec);
      push(h.netUp, s.net_up_per_sec);
      this.hist = { ...h };
      this.snap = s;
      this.error = null;
    } catch (e) {
      if (run !== this.#run) return;
      this.error = errorText(e);
    } finally {
      if (run === this.#run) {
        this.#pending = false;
        this.#timer = setTimeout(() => void this.#tick(), Math.max(0, PERIOD_MS - (performance.now() - t0)));
      }
    }
  }
}

export const nowSampler = new NowSampler();

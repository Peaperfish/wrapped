// A tiny frame-rate overlay for checking scroll smoothness. Off by default;
// ⌥⌘F toggles it and the choice is remembered. While off, no requestAnimationFrame
// loop runs and no element exists. Shows the average fps and the slowest frame
// over the last 2 seconds.
const KEY = "wrapped.frameMeter";
const WINDOW_MS = 2000;

let el: HTMLDivElement | null = null;
let raf = 0;

function stored(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

function remember(on: boolean) {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    /* private mode: not remembered */
  }
}

function start() {
  if (el) return;
  const box = document.createElement("div");
  box.className = "frame-meter";
  box.setAttribute("aria-hidden", "true");
  box.textContent = "… fps";
  document.body.appendChild(box);
  el = box;

  // [timestamp, duration since the previous frame]
  let frames: [number, number][] = [];
  let last = 0;
  let shown = 0;
  const tick = (t: number) => {
    if (last) frames.push([t, t - last]);
    last = t;
    const cutoff = t - WINDOW_MS;
    let drop = 0;
    while (drop < frames.length && frames[drop]![0] < cutoff) drop++;
    if (drop) frames = frames.slice(drop);
    // Update the text a few times a second, not every frame.
    if (t - shown > 250 && frames.length > 1) {
      shown = t;
      const span = frames[frames.length - 1]![0] - frames[0]![0];
      const fps = span > 0 ? ((frames.length - 1) * 1000) / span : 0;
      let worst = 0;
      for (const f of frames) if (f[1] > worst) worst = f[1];
      box.textContent = `${fps.toFixed(0)} fps · worst ${worst.toFixed(1)} ms`;
    }
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
}

function stop() {
  cancelAnimationFrame(raf);
  raf = 0;
  el?.remove();
  el = null;
}

export function initFrameMeter() {
  if (stored()) start();
  window.addEventListener("keydown", (e) => {
    if (e.metaKey && e.altKey && e.code === "KeyF") {
      e.preventDefault();
      const on = !el;
      if (on) start();
      else stop();
      remember(on);
    }
  });
}

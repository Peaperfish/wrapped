// Lock scrolling of the page (the document's scroll container) while a modal or
// popover is open, so wheel / touch scrolling never moves the dashboard behind
// it. Locks nest; the last release restores the overflow and scroll position.
let depth = 0;
let saved: { top: number; overflow: string; padRight: string } | null = null;

export function lockScroll(): () => void {
  const root = document.documentElement;
  const sc = document.scrollingElement ?? root;
  if (depth++ === 0) {
    saved = { top: sc.scrollTop, overflow: root.style.overflow, padRight: root.style.paddingRight };
    // Keep the layout from jumping when the scrollbar disappears.
    const bar = window.innerWidth - root.clientWidth;
    if (bar > 0) root.style.paddingRight = `${bar}px`;
    root.style.overflow = "hidden";
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    if (--depth === 0 && saved) {
      root.style.overflow = saved.overflow;
      root.style.paddingRight = saved.padRight;
      sc.scrollTop = saved.top;
      saved = null;
    }
  };
}

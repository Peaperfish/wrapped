// Which recaps have been opened (per machine, in localStorage). A recap is
// "New" in the list until it is opened once.
const KEY = "wrapped.recaps.seen";

function load(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return new Set(Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

let seen = load();

export const isSeen = (id: string): boolean => seen.has(id);

export function markSeen(id: string) {
  if (seen.has(id)) return;
  seen = new Set(seen).add(id);
  try {
    localStorage.setItem(KEY, JSON.stringify([...seen]));
  } catch {
    /* not remembered */
  }
}

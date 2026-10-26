import type { Counts } from "./types";

export function ago(ts: number, now = Date.now()): string {
  const min = Math.floor((now - ts) / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return h === 1 ? "1 hour ago" : `${h} hours ago`;
  const d = Math.floor(h / 24);
  if (d === 1) return "yesterday";
  if (d < 7) return `${d} days ago`;
  return new Date(ts).toLocaleDateString("en-US", { day: "numeric", month: "short" });
}

export function dateTime(ts: number): string {
  return new Date(ts).toLocaleString("en-US", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

/** Project recap, e.g. “2 working · 1 waiting for you”. */
export function countsSummary(c: Counts): string {
  const parts: string[] = [];
  if (c.waiting) parts.push(`${c.waiting} waiting for you`);
  if (c.working) parts.push(`${c.working} working`);
  if (c.finished) parts.push(`${c.finished} finished`);
  if (!parts.length) {
    const rest = c.open - c.waiting - c.working - c.finished;
    if (rest > 0) parts.push(`${rest} open`);
  }
  if (c.closed) parts.push(`${c.closed} closed`);
  return parts.join(" · ");
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Soft, stable color for the project's initial. */
export function projectHue(name: string): number {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return h;
}

/** Reorders keeping the positions already shown; new items go at the end. */
export function stableOrder<T>(items: T[], key: (t: T) => string | number, previous: (string | number)[]): T[] {
  const byKey = new Map(items.map((i) => [key(i), i]));
  const out: T[] = [];
  for (const k of previous) {
    const it = byKey.get(k);
    if (it) {
      out.push(it);
      byKey.delete(k);
    }
  }
  for (const it of items) if (byKey.has(key(it))) out.push(it);
  return out;
}

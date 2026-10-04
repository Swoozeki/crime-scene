export const num = (n: number | null | undefined, d = 0) =>
  n === null || n === undefined || Number.isNaN(n) ? '–' : n.toLocaleString(undefined, { maximumFractionDigits: d, minimumFractionDigits: d });

export const pct = (x: number | null | undefined, d = 0) => (x === null || x === undefined ? '–' : `${(x * 100).toFixed(d)}%`);

export const date = (ts: number | null | undefined) =>
  ts ? new Date(ts * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : '–';

export function ago(days: number | null | undefined): string {
  if (days === null || days === undefined) return '–';
  if (days < 1) return 'today';
  if (days < 60) return `${Math.round(days)}d ago`;
  if (days < 730) return `${Math.round(days / 30.4)}mo ago`;
  return `${(days / 365.25).toFixed(1)}y ago`;
}

export const agoTs = (now: number, ts: number | null | undefined) => (ts ? ago((now - ts) / 86400) : '–');

/** Last path segment, for compact labels. */
export const base = (p: string) => p.split('/').pop() ?? p;

/** Directory part of a path. */
export const dir = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '');

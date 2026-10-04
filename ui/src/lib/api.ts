// API client. In the live app it calls the csi server; in a static export it reads the
// responses baked into the page (window.__CSI_STATIC__), keyed by endpoint + sorted params.

declare global {
  interface Window {
    __CSI_STATIC__?: Record<string, unknown>;
  }
}

export const isStatic = typeof window !== 'undefined' && !!window.__CSI_STATIC__;

export type Params = Record<string, string | number | boolean | undefined | null>;

export function keyOf(endpoint: string, params: Params = {}): string {
  const entries = Object.entries(params)
    .filter(([, v]) => v !== undefined && v !== null && v !== '')
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([k, v]) => `${k}=${v}`);
  return entries.length ? `${endpoint}?${entries.join('&')}` : endpoint;
}

const cache = new Map<string, Promise<unknown>>();

export function api<T = any>(endpoint: string, params: Params = {}, opts: { fresh?: boolean } = {}): Promise<T> {
  const key = keyOf(endpoint, params);
  if (isStatic) {
    const data = window.__CSI_STATIC__![key];
    return data === undefined
      ? Promise.reject(new Error('Not included in this static report. Run `csi serve` for the full interactive app.'))
      : Promise.resolve(data as T);
  }
  if (!opts.fresh && cache.has(key)) return cache.get(key) as Promise<T>;
  const p = fetch(`/api/${key}`).then(async (r) => {
    const body = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(body.error ?? `HTTP ${r.status}`);
    return body as T;
  });
  p.catch(() => cache.delete(key));
  cache.set(key, p);
  return p;
}

export async function rescan(): Promise<any> {
  const r = await fetch('/api/scan', { method: 'POST' });
  const body = await r.json();
  if (!r.ok) throw new Error(body.error ?? `HTTP ${r.status}`);
  cache.clear();
  return body;
}

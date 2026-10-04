// Minimal hash router: works the same in the served app and in a file:// static export.

function parse() {
  const raw = location.hash.replace(/^#\/?/, '');
  const [path, query = ''] = raw.split('?');
  const parts = path.split('/').filter(Boolean);
  return { view: parts[0] ?? 'overview', arg: parts[1], query: Object.fromEntries(new URLSearchParams(query)) };
}

export const route = $state(parse());

window.addEventListener('hashchange', () => {
  const r = parse();
  route.view = r.view;
  route.arg = r.arg;
  route.query = r.query;
  window.scrollTo(0, 0);
});

export function href(view: string, arg?: string | number, query: Record<string, string | number | undefined> = {}) {
  const q = Object.entries(query).filter(([, v]) => v !== undefined && v !== '');
  const qs = q.length ? '?' + new URLSearchParams(q.map(([k, v]) => [k, String(v)])).toString() : '';
  return `#/${view}${arg !== undefined ? '/' + arg : ''}${qs}`;
}

export function go(view: string, arg?: string | number, query: Record<string, string | number | undefined> = {}) {
  location.hash = href(view, arg, query);
}

/** Update query params of the current route without adding history noise. */
export function setQuery(patch: Record<string, string | undefined>) {
  const next = { ...route.query, ...patch };
  for (const k of Object.keys(next)) if (next[k] === undefined || next[k] === '') delete next[k];
  history.replaceState(null, '', href(route.view, route.arg, next));
  route.query = next as Record<string, string>;
}

<script lang="ts">
  import { api, isStatic, rescan } from './lib/api';
  import { route, href } from './lib/router.svelte';
  import { filters, setRepo } from './lib/state.svelte';
  import Tooltip from './components/Tooltip.svelte';
  import Overview from './views/Overview.svelte';
  import Hotspots from './views/Hotspots.svelte';
  import MapView from './views/MapView.svelte';
  import Architecture from './views/Architecture.svelte';
  import Coupling from './views/Coupling.svelte';
  import Knowledge from './views/Knowledge.svelte';
  import Entity from './views/Entity.svelte';
  import Diff from './views/Diff.svelte';

  const meta = api<any>('meta');
  let scanning = $state(false);
  let scanMsg = $state('');
  let reloadKey = $state(0);

  const nav = [
    ['overview', 'Findings'],
    ['hotspots', 'Hotspots'],
    ['map', 'Hotspot map'],
    ['architecture', 'Architecture'],
    ['coupling', 'Coupling'],
    ['knowledge', 'Knowledge'],
    ['diff', 'Change risk'],
  ];

  let theme = $state<string>((() => {
    try {
      return localStorage.getItem('csi.theme') ?? '';
    } catch {
      return '';
    }
  })());
  $effect(() => {
    if (theme) document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
    try {
      localStorage.setItem('csi.theme', theme);
    } catch {
      /* ignore */
    }
  });

  async function doScan() {
    scanning = true;
    scanMsg = 'Scanning…';
    try {
      const r = await rescan();
      const n = r.repos.reduce((a: number, x: any) => a + x.new_commits, 0);
      scanMsg = `${n} new commit${n === 1 ? '' : 's'} · ${r.seconds.toFixed(1)}s`;
      reloadKey++;
    } catch (e) {
      scanMsg = String(e);
    } finally {
      scanning = false;
    }
  }
</script>

<div class="shell">
  <aside>
    <a class="brand" href={href('overview')}>
      <svg width="22" height="22" viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="13" fill="none" stroke="var(--accent)" stroke-width="3" /><circle cx="16" cy="16" r="5" fill="var(--accent)" /></svg>
      <span>csi</span>
    </a>
    {#await meta then m}
      <div class="ws" title="workspace">{m.workspace}</div>
    {/await}
    <nav>
      {#each nav as [v, label]}
        <a href={href(v)} class:on={route.view === v || (v === 'hotspots' && route.view === 'entity')}>{label}</a>
      {/each}
    </nav>
    <div class="side-foot">
      {#await meta then m}
        {#if m.repos.length > 1}
          <label class="muted small" for="repo">Repository</label>
          <select id="repo" value={filters.repo} onchange={(e) => setRepo((e.target as HTMLSelectElement).value)}>
            <option value="">All repos ({m.repos.length})</option>
            {#each m.repos as r}<option value={r.name}>{r.name}</option>{/each}
          </select>
        {/if}
      {/await}
      {#if !isStatic}
        <button onclick={doScan} disabled={scanning}>{scanning ? 'Scanning…' : 'Rescan'}</button>
        {#if scanMsg}<div class="muted small">{scanMsg}</div>{/if}
      {:else}
        <div class="muted small">Static report — run <code>csi serve</code> for live analysis.</div>
      {/if}
      <div class="seg theme" role="group" aria-label="Theme">
        <button class:on={theme === ''} onclick={() => (theme = '')}>Auto</button>
        <button class:on={theme === 'light'} onclick={() => (theme = 'light')}>Light</button>
        <button class:on={theme === 'dark'} onclick={() => (theme = 'dark')}>Dark</button>
      </div>
    </div>
  </aside>
  <main>
    {#key `${reloadKey}|${filters.repo}|${theme}`}
      {#if route.view === 'overview'}<Overview />
      {:else if route.view === 'hotspots'}<Hotspots />
      {:else if route.view === 'map'}<MapView />
      {:else if route.view === 'architecture'}<Architecture />
      {:else if route.view === 'coupling'}<Coupling />
      {:else if route.view === 'knowledge'}<Knowledge />
      {:else if route.view === 'entity'}{#key route.arg + (route.query.level ?? '')}<Entity />{/key}
      {:else if route.view === 'diff'}<Diff />
      {:else}<div class="empty">Unknown page.</div>{/if}
    {/key}
  </main>
</div>
<Tooltip />

<style>
  .shell { display: grid; grid-template-columns: 210px minmax(0, 1fr); min-height: 100vh; }
  aside { position: sticky; top: 0; height: 100vh; display: flex; flex-direction: column; gap: 6px; padding: 18px 14px; border-right: 1px solid var(--border); background: var(--surface); }
  .brand { display: flex; align-items: center; gap: 8px; font-weight: 700; font-size: 18px; color: var(--ink); letter-spacing: -0.02em; }
  .brand:hover { text-decoration: none; }
  .ws { color: var(--muted); font-size: 12.5px; margin: 0 0 12px 30px; overflow: hidden; text-overflow: ellipsis; }
  nav { display: flex; flex-direction: column; gap: 2px; }
  nav a { color: var(--ink-2); padding: 7px 10px; border-radius: 7px; }
  nav a:hover { background: var(--surface-2); text-decoration: none; color: var(--ink); }
  nav a.on { background: var(--surface-2); color: var(--ink); font-weight: 600; box-shadow: inset 2px 0 0 var(--accent); }
  .side-foot { margin-top: auto; display: flex; flex-direction: column; gap: 8px; }
  .small { font-size: 12px; }
  .theme { align-self: flex-start; }
  .theme button { padding: 3px 8px; font-size: 12px; }
  main { padding: 24px 28px 60px; min-width: 0; max-width: 1500px; }
  @media (max-width: 760px) {
    .shell { grid-template-columns: 1fr; }
    aside { position: static; height: auto; border-right: 0; border-bottom: 1px solid var(--border); }
    nav { flex-direction: row; flex-wrap: wrap; }
    main { padding: 16px; }
  }
</style>

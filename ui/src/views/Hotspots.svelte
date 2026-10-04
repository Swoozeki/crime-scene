<script lang="ts">
  import { api } from '../lib/api';
  import { route, href, setQuery, go } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import { num, ago } from '../lib/fmt';
  import ScoreBar from '../components/ScoreBar.svelte';
  import Health from '../components/Health.svelte';
  import Trend from '../components/Trend.svelte';
  import Path from '../components/Path.svelte';

  const level = $derived(route.query.level ?? 'entity');
  const rows = $derived(api<any[]>('hotspots', { level }));
  const meta = api<any>('meta');
  let q = $state(route.query.q ?? '');
  let unit = $state(route.query.unit ?? '');
  let sortKey = $state<string>('rank');
  let asc = $state(true);
  let limit = $state(200);

  const cols: [string, string, boolean][] = [
    ['rank', '#', true],
    ['score', 'Score', false],
    ['name', 'Name', true],
    ['revisions', 'Changes', false],
    ['complexity', 'Complexity', false],
    ['loc', 'Lines', false],
    ['health', 'Health', true],
    ['main_dev', 'Main developer', true],
    ['age_days', 'Last change', true],
  ];

  function sortBy(k: string, defAsc: boolean) {
    if (sortKey === k) asc = !asc;
    else {
      sortKey = k;
      asc = defAsc;
    }
  }

  function filtered(all: any[]) {
    const needle = q.toLowerCase();
    let r = all.filter(
      (h) =>
        (!filters.repo || h.repo === filters.repo) &&
        (!unit || h.unit === unit || h.unit?.endsWith(':' + unit) || (level === 'unit' && h.name === unit)) &&
        (!needle || h.name.toLowerCase().includes(needle)),
    );
    const dir = asc ? 1 : -1;
    r = [...r].sort((a, b) => {
      const x = a[sortKey] ?? (typeof b[sortKey] === 'string' ? '' : -Infinity);
      const y = b[sortKey] ?? (typeof a[sortKey] === 'string' ? '' : -Infinity);
      return (x < y ? -1 : x > y ? 1 : 0) * dir;
    });
    return r;
  }

  function open(h: any) {
    if (level === 'file' || level === 'entity') go('entity', h.key, { level });
    else if (level === 'unit') go('hotspots', undefined, { level: 'entity', unit: h.unit ?? h.name });
    else go('hotspots', undefined, { level: 'unit' });
  }
</script>

<header class="row wrap">
  <div class="grow">
    <h1>Hotspots</h1>
    <div class="muted">Code that changes often <i>and</i> is complex. Change frequency is recency-weighted; complexity is normalized per language.</div>
  </div>
</header>

<div class="controls row wrap">
  <div class="seg" role="group" aria-label="Level">
    {#each [['file', 'Files'], ['entity', 'Components & classes'], ['unit', 'Units'], ['repo', 'Repos']] as [l, label]}
      <button class:on={level === l} onclick={() => setQuery({ level: l })}>{label}</button>
    {/each}
  </div>
  <input type="search" placeholder="Filter by name…" bind:value={q} oninput={() => setQuery({ q })} />
  {#await meta then m}
    {#if level === 'file' || level === 'entity'}
      <select bind:value={unit} onchange={() => setQuery({ unit })} aria-label="Unit">
        <option value="">All units</option>
        {#each m.units.filter((u: any) => !filters.repo || u.repo === filters.repo) as u}
          <option value={m.repos.length > 1 ? `${u.repo}:${u.name}` : u.name}>{m.repos.length > 1 ? `${u.repo}:` : ''}{u.name} ({u.kind})</option>
        {/each}
      </select>
    {/if}
  {/await}
</div>

{#await rows}
  <div class="empty">Ranking…</div>
{:then all}
  {@const list = filtered(all)}
  <div class="card tablewrap">
    <table class="data">
      <thead>
        <tr>
          {#each cols as [k, label, defAsc]}
            <th class="sortable" class:num={['rank', 'revisions', 'complexity', 'loc'].includes(k)} onclick={() => sortBy(k, defAsc)}>
              {label}{sortKey === k ? (asc ? ' ↑' : ' ↓') : ''}
            </th>
          {/each}
          <th>Trend</th>
        </tr>
      </thead>
      <tbody>
        {#each list.slice(0, limit) as h (h.key)}
          <tr class="click" onclick={() => open(h)}>
            <td class="num muted">{h.rank}</td>
            <td style="white-space:nowrap"><ScoreBar value={h.score} /></td>
            <td class="name">
              <a href={level === 'file' || level === 'entity' ? href('entity', h.key, { level }) : undefined} onclick={(e) => e.preventDefault()}>
                <Path path={h.name} repo={!filters.repo && all.some((x) => x.repo !== h.repo) ? h.repo : ''} />
              </a>
              {#if h.kind && !['file', 'dir', 'repo'].includes(h.kind)}<span class="pill">{h.kind}</span>{/if}
              {#if h.test}<span class="pill">test</span>{/if}
            </td>
            <td class="num" title="{h.rev_w.toFixed(1)} recency-weighted">{num(h.revisions)}</td>
            <td class="num">{num(h.complexity)}</td>
            <td class="num">{num(h.loc)}</td>
            <td><Health value={h.health} /></td>
            <td class="ellipsis dev">{h.main_dev ?? '–'}{#if h.main_dev}<span class="muted"> {Math.round(h.main_dev_share * 100)}%</span>{/if}</td>
            <td class="muted">{ago(h.age_days)}</td>
            <td><Trend trend={h.trend} /></td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if list.length === 0}<div class="empty">Nothing matches.</div>{/if}
  </div>
  {#if list.length > limit}
    <button class="more" onclick={() => (limit += 300)}>Show more ({list.length - limit} remaining)</button>
  {/if}
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header { margin-bottom: 14px; }
  .controls { margin-bottom: 14px; }
  .controls input { width: 240px; }
  .tablewrap { overflow: auto; max-height: calc(100vh - 190px); }
  td.name { max-width: 520px; display: flex; gap: 8px; align-items: center; }
  td.name a { min-width: 0; color: var(--ink); }
  .dev { max-width: 200px; }
  .more { margin-top: 12px; }
</style>

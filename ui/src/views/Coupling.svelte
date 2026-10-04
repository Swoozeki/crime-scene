<script lang="ts">
  import { api } from '../lib/api';
  import { route, setQuery, go } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import Path from '../components/Path.svelte';

  const level = $derived(route.query.level ?? 'entity');
  const by = $derived(route.query.by ?? 'commit');
  const tests = $derived(route.query.tests === '1');
  const config = $derived(route.query.config === '1');
  const rows = $derived(
    api<any[]>('coupling', { by, level, ...(tests ? { tests: 'true' } : {}), ...(config ? { config: 'true' } : {}) }),
  );
  let cross = $state(route.query.cross === '1');
  let q = $state(route.query.q ?? '');
  let limit = $state(200);

  function filtered(all: any[]) {
    const n = q.toLowerCase();
    return all.filter(
      (c) =>
        (!filters.repo || c.a_repo === filters.repo || c.b_repo === filters.repo) &&
        (!cross || c.cross_unit || c.cross_repo) &&
        (!n || c.a_name.toLowerCase().includes(n) || c.b_name.toLowerCase().includes(n)),
    );
  }

  const multi = (all: any[]) => all.some((c) => c.a_repo !== c.b_repo || c.a_repo !== all[0]?.a_repo);
</script>

<header>
  <h1>Change coupling</h1>
  <div class="muted">Pairs that change together more often than chance (lift &gt; threshold). With squash merges a commit is a PR; <b>by ticket</b> joins all commits sharing a ticket ID — across repositories.</div>
</header>

<div class="row wrap controls">
  <div class="seg" role="group" aria-label="Level">
    {#each [['file', 'Files'], ['entity', 'Components & classes'], ['unit', 'Units']] as [l, label]}
      <button class:on={level === l} onclick={() => setQuery({ level: l })}>{label}</button>
    {/each}
  </div>
  <div class="seg" role="group" aria-label="Group changes by">
    <button class:on={by === 'commit'} onclick={() => setQuery({ by: 'commit' })}>By commit / PR</button>
    <button class:on={by === 'ticket'} onclick={() => setQuery({ by: 'ticket' })}>By ticket</button>
  </div>
  <label class="row"><input type="checkbox" bind:checked={cross} onchange={() => setQuery({ cross: cross ? '1' : '' })} /> Only across unit / repo boundaries</label>
  <label class="row"><input type="checkbox" checked={tests} onchange={(e) => setQuery({ tests: e.currentTarget.checked ? '1' : '' })} /> Include tests</label>
  <label class="row"><input type="checkbox" checked={config} onchange={(e) => setQuery({ config: e.currentTarget.checked ? '1' : '' })} /> Include config files</label>
  <input type="search" placeholder="Filter…" bind:value={q} oninput={() => setQuery({ q })} />
</div>

{#await rows}
  <div class="empty">Counting co-changes…</div>
{:then all}
  {@const list = filtered(all)}
  {@const showRepo = multi(all)}
  <div class="card tablewrap">
    <table class="data">
      <thead>
        <tr><th>A</th><th>B</th><th class="num">Together</th><th>Strength</th><th class="num">Lift</th><th>Boundary</th></tr>
      </thead>
      <tbody>
        {#each list.slice(0, limit) as c}
          <tr class="click" onclick={() => level !== 'unit' && go('entity', c.a, { level })}>
            <td class="nm"><Path path={c.a_name} repo={showRepo ? c.a_repo : ''} /></td>
            <td class="nm"><Path path={c.b_name} repo={showRepo ? c.b_repo : ''} /></td>
            <td class="num" title="{c.recent} in the last 180 days">{c.support} <span class="muted">({c.recent} recent)</span></td>
            <td>
              <div class="conf" title="share of A's changes that also change B, and vice versa">
                <span class="bar"><span style="width:{c.conf_ab * 100}%"></span></span><span class="num">A→B {Math.round(c.conf_ab * 100)}%</span>
                <span class="bar"><span style="width:{c.conf_ba * 100}%"></span></span><span class="num">B→A {Math.round(c.conf_ba * 100)}%</span>
              </div>
            </td>
            <td class="num">{c.lift.toFixed(1)}</td>
            <td>
              {#if c.cross_repo}<span class="pill x">cross-repo</span>{:else if c.cross_unit}<span class="pill">cross-unit</span>{/if}
              {#if c.test_pair}<span class="pill">test</span>{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if list.length === 0}<div class="empty">No coupling above the thresholds{by === 'commit' ? ' — try “By ticket”' : ''}.</div>{/if}
  </div>
  {#if list.length > limit}<button class="more" onclick={() => (limit += 300)}>Show more</button>{/if}
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header, .controls { margin-bottom: 14px; }
  .tablewrap { overflow: auto; max-height: calc(100vh - 200px); }
  .nm { max-width: 380px; }
  .conf { display: grid; grid-template-columns: 80px auto; gap: 2px 8px; align-items: center; font-size: 12px; }
  .bar { height: 6px; background: var(--grid); border-radius: 3px; overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--link); border-radius: 3px; }
  .x { border-color: var(--accent); color: var(--accent-ink); }
  .more { margin-top: 12px; }
</style>

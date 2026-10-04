<script lang="ts">
  import { api } from '../lib/api';
  import { go } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import { personScale, topPeople } from '../lib/colors';
  import { pct } from '../lib/fmt';
  import MapView from './MapView.svelte';

  const owners = Promise.all([api<any[]>('owners', { level: 'unit' }), api<any>('tree')]);

  function scopedTree(t: any) {
    return filters.repo ? { ...t, children: t.children.filter((c: any) => c.name === filters.repo) } : t;
  }
</script>

<MapView initialMode="dev" title="Knowledge map" />

<h2 class="sec">Units by knowledge risk</h2>
<p class="muted">Ownership is lines added per author. <b>Bus factor</b> = fewest people holding over half the knowledge. <b>Knowledge loss</b> = share written by authors inactive for the configured period (†).</p>
{#await owners}
  <div class="empty">Loading…</div>
{:then [rows, tree]}
  {@const list = rows.filter((r) => !filters.repo || r.repo === filters.repo)}
  {@const ppl = topPeople(scopedTree(tree))}
  {@const color = personScale(ppl)}
  <div class="card tablewrap">
    <table class="data">
      <thead><tr><th>Unit</th><th>Knowledge distribution</th><th class="num">Bus factor</th><th class="num">Knowledge loss</th><th class="num">Fragmentation</th><th class="num">Active (1y)</th></tr></thead>
      <tbody>
        {#each list as o}
          <tr class="click" onclick={() => go('hotspots', undefined, { level: 'entity', unit: o.unit ?? o.name })}>
            <td>{o.unit ?? o.name}</td>
            <td>
              <div class="dist" aria-label="knowledge shares">
                {#each o.authors as a}
                  <span style="width:{a.share * 100}%;background:{color(a.name)};opacity:{a.active ? 1 : 0.45}" title="{a.name}{a.active ? '' : ' (inactive)'} {Math.round(a.share * 100)}%"></span>
                {/each}
              </div>
              <div class="who">{o.authors.slice(0, 3).map((a: any) => `${a.name}${a.active ? '' : '†'} ${Math.round(a.share * 100)}%`).join(' · ')}</div>
            </td>
            <td class="num" class:warn={o.bus_factor === 1}>{o.bus_factor}</td>
            <td class="num" class:warn={o.knowledge_loss >= 0.5}>{pct(o.knowledge_loss)}</td>
            <td class="num">{o.fragmentation.toFixed(2)}</td>
            <td class="num">{o.recent_authors}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  .sec { margin: 28px 0 4px; }
  .tablewrap { overflow: auto; max-height: 70vh; }
  .dist { display: flex; gap: 2px; height: 10px; width: 260px; background: var(--grid); border-radius: 3px; overflow: hidden; }
  .dist span { display: block; height: 100%; }
  .who { font-size: 11.5px; color: var(--ink-2); margin-top: 3px; }
  .warn { color: var(--critical); font-weight: 600; }
</style>

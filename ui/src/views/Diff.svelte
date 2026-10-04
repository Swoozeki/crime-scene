<script lang="ts">
  import { api, isStatic } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import { pct } from '../lib/fmt';
  import { STATUS } from '../lib/colors';
  import Path from '../components/Path.svelte';
  import Health from '../components/Health.svelte';

  const meta = api<any>('meta');
  let repo = $state(filters.repo);
  let base = $state('');
  let head = $state('');
  let ticket = $state('');
  let mode = $state<'range' | 'ticket'>('range');
  let result = $state<Promise<any> | null>(null);
  let copied = $state(false);

  function run(e: Event) {
    e.preventDefault();
    result = mode === 'ticket' ? api('diff', { ticket }, { fresh: true }) : api('diff', { repo, base, head }, { fresh: true });
  }

  const levelColor = (l: string) => (l === 'high' ? STATUS.critical : l === 'medium' ? STATUS.warning : STATUS.good);

  async function copy(md: string) {
    try {
      await navigator.clipboard.writeText(md);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* clipboard unavailable */
    }
  }
</script>

<header>
  <h1>Change risk</h1>
  <div class="muted">Review a branch or PR before merging: touched hotspots, complexity added, files that usually change together but were left out, and who should review. The same report is available as <code>csi diff --format md</code> for CI comments.</div>
</header>

{#if isStatic}
  <div class="card empty">Change risk needs the live app: run <code>csi serve</code> or <code>csi diff</code>.</div>
{:else}
  <form class="card pad row wrap" onsubmit={run}>
    <div class="seg" role="group" aria-label="Mode">
      <button type="button" class:on={mode === 'range'} onclick={() => (mode = 'range')}>Git range</button>
      <button type="button" class:on={mode === 'ticket'} onclick={() => (mode = 'ticket')}>Ticket</button>
    </div>
    {#if mode === 'range'}
      {#await meta then m}
        <select bind:value={repo} aria-label="Repository">
          {#each m.repos as r}<option value={r.name}>{r.name}</option>{/each}
        </select>
      {/await}
      <input type="text" bind:value={base} placeholder="base (default: merge-base)" aria-label="Base" />
      <span class="muted">..</span>
      <input type="text" bind:value={head} placeholder="head (default: HEAD)" aria-label="Head" />
    {:else}
      <input type="text" bind:value={ticket} placeholder="e.g. SHOP-1234" aria-label="Ticket" required />
    {/if}
    <button class="primary" type="submit">Assess</button>
  </form>

  {#if result}
    {#await result}
      <div class="empty">Assessing…</div>
    {:then r}
      <section class="card pad verdict">
        <div class="risk" style="--c:{levelColor(r.level)}"><b>{r.risk}</b><span>/100</span></div>
        <div class="grow">
          <div class="lvl"><span class="dot" style="background:{levelColor(r.level)}"></span>{r.level} risk · {r.title}</div>
          <ul>{#each r.reasons as x}<li>{x}</li>{/each}</ul>
          {#if r.reviewers.length}<div><b>Suggested reviewers:</b> {r.reviewers.map((e: any) => e.name).join(', ')}</div>{/if}
        </div>
        <button onclick={() => copy(r.markdown)}>{copied ? 'Copied' : 'Copy as Markdown'}</button>
      </section>

      {#if r.missed.length}
        <section class="card pad">
          <h3>Usually changed together — but not in this change</h3>
          <table class="data">
            <tbody>
              {#each r.missed as m}
                <tr>
                  <td><Path path={m.path} repo={m.kind === 'cross-repo' ? m.repo : ''} /></td>
                  <td class="num">{pct(m.confidence)}</td>
                  <td class="muted">with {m.because_of.split('/').pop()} · {m.support}×</td>
                  <td><span class="pill">{m.kind}</span></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/if}

      <section class="card pad">
        <h3>Files</h3>
        <table class="data">
          <thead><tr><th>File</th><th class="num">+/−</th><th>Hotspot</th><th>Complexity</th><th>Health</th><th>Functions touched</th></tr></thead>
          <tbody>
            {#each [...r.files].sort((a, b) => b.hotspot_score - a.hotspot_score) as f}
              <tr>
                <td><Path path={f.path} repo={r.repos.length > 1 ? f.repo : ''} /></td>
                <td class="num">+{f.added} −{f.deleted}</td>
                <td>{f.hotspot_score > 0 ? `#${f.hotspot_rank} (${f.hotspot_score.toFixed(2)})` : ''}</td>
                <td class="num">
                  {#if f.complexity_before !== null && f.complexity_after !== null && f.complexity_before !== f.complexity_after}
                    {f.complexity_before.toFixed(0)} → <b class:up={f.complexity_after > f.complexity_before}>{f.complexity_after.toFixed(0)}</b>
                  {:else}{f.complexity_after?.toFixed(0) ?? '–'}{/if}
                </td>
                <td><Health value={f.health_after} /></td>
                <td class="mono small">{f.functions.slice(0, 4).map((x: any) => x.name + (x.cc_before !== null && x.cc_after !== x.cc_before ? ` (cc ${x.cc_before}→${x.cc_after})` : '')).join(', ')}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {:catch e}
      <div class="card error">{e.message}</div>
    {/await}
  {/if}
{/if}

<p class="muted small foot">Tip: <a href={href('coupling', undefined, { by: 'ticket' })}>ticket coupling</a> shows what usually ships together across repositories.</p>

<style>
  header { margin-bottom: 14px; }
  form { gap: 10px; margin-bottom: 14px; }
  form input { width: 220px; }
  section.card { margin-bottom: 14px; }
  .verdict { display: flex; gap: 20px; align-items: flex-start; }
  .risk { width: 86px; height: 86px; border-radius: 50%; border: 6px solid var(--c); display: grid; place-content: center; text-align: center; flex: none; }
  .risk b { font-size: 26px; line-height: 1; }
  .risk span { font-size: 11px; color: var(--muted); }
  .lvl { font-weight: 600; text-transform: capitalize; display: flex; gap: 8px; align-items: center; }
  ul { margin: 8px 0; padding-left: 18px; color: var(--ink-2); }
  h3 { margin-bottom: 8px; }
  .up { color: var(--critical); }
  .small { font-size: 12px; }
  .foot { margin-top: 20px; }
</style>

<script lang="ts">
  import { api } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import { num, pct, date } from '../lib/fmt';
  import { heat } from '../lib/colors';

  const data = Promise.all([api<any>('overview'), api<any[]>('findings')]);
  let kind = $state('');
  let open = $state<Record<string, boolean>>({});

  const KIND_LABEL: Record<string, string> = {
    hotspot: 'Hotspot',
    deteriorating_hotspot: 'Deteriorating hotspot',
    xray_hotspot: 'Function hotspot',
    hidden_coupling: 'Hidden coupling',
    shotgun_surgery: 'Ripple effect',
    knowledge_loss: 'Knowledge loss',
    bus_factor: 'Bus factor',
    coordination: 'Coordination',
    defect_magnet: 'Defect magnet',
    bloated_component: 'Bloated component',
  };

  function link(f: any): string {
    if (f.level === 'unit') return href('hotspots', undefined, { level: 'entity', unit: f.unit ?? '' });
    return href('entity', f.key, { level: f.level });
  }

  function evidenceRows(ev: Record<string, unknown>): [string, string][] {
    return Object.entries(ev)
      .filter(([, v]) => v !== null && v !== undefined && !(Array.isArray(v) && v.length === 0))
      .map(([k, v]) => [k.replace(/_/g, ' '), typeof v === 'object' ? JSON.stringify(v, null, 1).replace(/[{}"[\]]/g, '').trim() : String(v)]);
  }
</script>

{#await data}
  <div class="empty">Loading the crime scene…</div>
{:then [o, findings]}
  {@const repoF = findings.filter((f) => !filters.repo || f.repo === filters.repo)}
  {@const shown = repoF.filter((f) => !kind || f.kind === kind)}
  {@const kinds = [...new Set(repoF.map((f) => f.kind))]}
  <header class="row wrap">
    <div class="grow">
      <h1>{o.workspace}</h1>
      <div class="muted">
        {o.repos.length} repo{o.repos.length === 1 ? '' : 's'} · {num(o.commits)} commits · {date(o.first_commit)} – {date(o.last_commit)}
        · recency half-life {o.config.half_life_days} days
      </div>
    </div>
  </header>

  <section class="tiles">
    <div class="tile card"><div class="v">{num(o.files)}</div><div class="l">files · {num(o.loc)} lines</div></div>
    <div class="tile card"><div class="v">{o.active_authors}<span class="muted small"> / {o.authors}</span></div><div class="l">active authors</div></div>
    <div class="tile card" title="Share of recency-weighted change that lands in the top 5% of entities">
      <div class="v">{pct(o.hotspot_concentration)}</div><div class="l">of change hits the top 5%</div>
    </div>
    <div class="tile card" title="Average share of code written by authors inactive for {o.config.inactive_after_days} days">
      <div class="v">{pct(o.knowledge_loss)}</div><div class="l">knowledge loss</div>
    </div>
    <div class="tile card"><div class="v">{o.bus_factor_one_units}</div><div class="l">units with bus factor 1</div></div>
    <div class="tile card"><div class="v">{o.cross_unit_couplings}{#if o.repos.length > 1}<span class="muted small"> + {o.cross_repo_couplings} cross-repo</span>{/if}</div><div class="l">strong couplings across units</div></div>
  </section>

  <section class="repos">
    {#each o.repos.filter((r: any) => !filters.repo || r.name === filters.repo) as r}
      <div class="repo">
        <b>{r.name}</b> <span class="muted mono">{r.branch}</span>
        <span class="muted">· {num(r.commits)} commits · {num(r.files)} files · {r.units} units</span>
        {#each r.frameworks as f}<span class="pill">{f}</span>{/each}
      </div>
    {/each}
  </section>

  <section>
    <div class="row wrap head">
      <h2 class="grow">Findings <span class="muted">({shown.length})</span></h2>
      <div class="chips">
        <button class="chip" class:on={kind === ''} onclick={() => (kind = '')}>All</button>
        {#each kinds as k}
          <button class="chip" class:on={kind === k} onclick={() => (kind = k)}>{KIND_LABEL[k] ?? k}</button>
        {/each}
      </div>
    </div>
    {#if shown.length === 0}
      <div class="card empty">No findings. Nothing stands out — check <a href={href('hotspots')}>Hotspots</a> for the full ranking.</div>
    {/if}
    <ol class="findings">
      {#each shown as f (f.id)}
        <li class="card finding">
          <div class="sev" style="--c:{heat(f.severity / 100)}" title="severity {f.severity}/100"><span>{f.severity}</span></div>
          <div class="body">
            <div class="meta"><span class="kind">{f.kind_label ?? KIND_LABEL[f.kind] ?? f.kind}</span> <span class="muted">· {f.id}{f.unit ? ' · ' + f.unit : ''}</span></div>
            <a class="title" href={link(f)}>{f.title}</a>
            <p class="rec">{f.recommendation}</p>
            <button class="link" onclick={() => (open[f.id] = !open[f.id])}>{open[f.id] ? 'Hide evidence' : 'Evidence'}</button>
            {#if open[f.id]}
              <dl class="ev">
                {#each evidenceRows(f.evidence) as [k, v]}<dt>{k}</dt><dd class="mono">{v}</dd>{/each}
              </dl>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
  </section>
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header { margin-bottom: 18px; }
  .small { font-size: 13px; font-weight: 400; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 12px; margin-bottom: 14px; }
  .tile { padding: 14px 16px; }
  .tile .v { font-size: 26px; font-weight: 600; letter-spacing: -0.02em; }
  .tile .l { color: var(--ink-2); font-size: 12.5px; }
  .repos { display: flex; flex-direction: column; gap: 4px; margin-bottom: 26px; font-size: 13px; }
  .repo { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .head { margin-bottom: 12px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { border-radius: 99px; padding: 3px 11px; font-size: 12.5px; color: var(--ink-2); }
  .chip.on { background: var(--ink); color: var(--surface); border-color: var(--ink); }
  .findings { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 10px; }
  .finding { display: grid; grid-template-columns: 54px 1fr; gap: 0; overflow: hidden; }
  .sev { display: flex; align-items: flex-start; justify-content: center; padding-top: 16px; border-right: 1px solid var(--border); }
  .sev span { display: inline-grid; place-items: center; width: 36px; height: 36px; border-radius: 50%; border: 3px solid var(--c); font-weight: 700; font-variant-numeric: tabular-nums; font-size: 13px; }
  .body { padding: 13px 16px; min-width: 0; }
  .meta { font-size: 12px; }
  .kind { font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; font-size: 11px; color: var(--accent-ink); }
  .title { display: block; color: var(--ink); font-weight: 600; font-size: 15px; margin: 2px 0 4px; overflow-wrap: anywhere; }
  .rec { margin: 0 0 6px; color: var(--ink-2); }
  .link { border: 0; background: none; padding: 0; color: var(--link); font-size: 12.5px; }
  .link:hover { text-decoration: underline; background: none; }
  .ev { display: grid; grid-template-columns: max-content 1fr; gap: 3px 14px; margin: 8px 0 0; font-size: 12.5px; }
  .ev dt { color: var(--muted); }
  .ev dd { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--ink-2); }
</style>

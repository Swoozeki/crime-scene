<script lang="ts">
  import { api } from '../lib/api';
  import { route, href } from '../lib/router.svelte';
  import { num, pct, date, agoTs, base } from '../lib/fmt';
  import { heat, personScale, topPeople } from '../lib/colors';
  import ScoreBar from '../components/ScoreBar.svelte';
  import Health from '../components/Health.svelte';
  import Trend from '../components/Trend.svelte';
  import TrendChart from '../components/TrendChart.svelte';
  import Path from '../components/Path.svelte';

  const level = route.query.level ?? 'entity';
  const key = Number(route.arg);
  const data = Promise.all([api<any>('entity', { key, level }), api<any>('meta'), api<any>('tree').catch(() => null)]);

  const maxOf = (xs: number[]) => Math.max(1, ...xs);
</script>

{#await data}
  <div class="empty">Investigating… (first visit computes function-level history)</div>
{:then [d, meta, tree]}
  {@const h = d.hotspot}
  {@const color = personScale(tree ? topPeople(tree) : (d.ownership?.authors ?? []).map((a: any) => a.name))}
  <header>
    <div class="crumbs muted"><a href={href('hotspots', undefined, { level })}>Hotspots</a> / {d.repo}{h?.unit ? ' / ' + h.unit : ''}</div>
    <h1 class="mono title">{d.name}</h1>
    <div class="row wrap tags">
      {#if h?.kind && h.kind !== 'file'}<span class="pill">{h.kind}</span>{/if}
      {#if h?.test}<span class="pill">test code</span>{/if}
      <Trend trend={h?.trend} />
      {#if level === 'file'}
        {@const f = d.files.find((x: any) => x.key === key)}
        {#if f}<span class="pill">{f.lang}</span>{/if}
      {/if}
    </div>
  </header>

  {#if h}
    <section class="stats">
      <div class="stat"><div class="l">Hotspot rank</div><div class="v">#{h.rank} <span class="muted small">of {num(d.total_ranked)}</span></div><ScoreBar value={h.score} width={90} /></div>
      <div class="stat"><div class="l">Changes</div><div class="v">{num(h.revisions)}</div><div class="muted small">{h.rev_w.toFixed(1)} recency-weighted · last {agoTs(meta.now, h.last_change)}</div></div>
      <div class="stat"><div class="l">Complexity</div><div class="v">{num(h.complexity)}</div><div class="muted small">{num(h.loc)} lines · {h.files} file{h.files === 1 ? '' : 's'}</div></div>
      <div class="stat"><div class="l">Code health</div><div class="v"><Health value={h.health} label /></div><div class="muted small">{h.health_reasons.length} issue{h.health_reasons.length === 1 ? '' : 's'}</div></div>
      <div class="stat"><div class="l">Defect fixes</div><div class="v">{h.defects}</div><div class="muted small">{pct(h.defect_density)} of changes</div></div>
      <div class="stat"><div class="l">Authors</div><div class="v">{h.authors}</div><div class="muted small">{h.active_authors} active</div></div>
    </section>
  {:else}
    <div class="card pad muted">No changes to this code in the analyzed window.</div>
  {/if}

  {#if h?.health_reasons.length}
    <section class="card pad reasons">
      <h3>Why the health score</h3>
      <ul>{#each h.health_reasons as r}<li>{r}</li>{/each}</ul>
    </section>
  {/if}

  <div class="grid">
    <section class="card pad">
      <h3>Complexity over time</h3>
      <TrendChart points={d.trend} />
    </section>

    <section class="card pad">
      <h3>Files</h3>
      <table class="data compact">
        <tbody>
          {#each d.files as f}
            <tr>
              <td><a href={href('entity', f.key, { level: 'file' })}><Path path={base(f.path)} /></a> {#if f.test}<span class="pill">test</span>{/if}</td>
              <td class="muted">{f.lang}</td>
              <td class="num">{num(f.loc)} <span class="muted">loc</span></td>
              <td class="num">{num(f.complexity)} <span class="muted">cx</span></td>
              <td><Health value={f.health} /></td>
            </tr>
            {#if f.angular}
              <tr class="ng"><td colspan="5" class="muted">Angular {f.angular.kind} · {f.angular.deps} injected deps · {f.angular.inputs} inputs · {f.angular.outputs} outputs{f.angular.standalone ? ' · standalone' : ''}</td></tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </section>
  </div>

  {#if d.xray && d.xray.functions.length}
    {@const fns = d.xray.functions.slice(0, 20)}
    {@const maxRev = maxOf(fns.map((f: any) => f.revisions))}
    <section class="card pad">
      <div class="row"><h3 class="grow">X-ray — functions by change × complexity</h3><span class="muted small mono">{d.xray.path} · {d.xray.analyzed_commits} commits analyzed</span></div>
      <table class="data compact xray">
        <thead><tr><th>Function</th><th>Changes</th><th class="num">CC</th><th class="num">Nesting</th><th class="num">Lines</th><th class="num">Authors</th><th>Last change</th></tr></thead>
        <tbody>
          {#each fns as f}
            <tr>
              <td class="mono fn" title={f.name}>{f.name}</td>
              <td class="barcell">
                <span class="hbar" style="width:{(f.revisions / maxRev) * 140}px;background:{heat(0.2 + f.score * 0.8)}"></span>
                <span class="num">{f.revisions}</span>
              </td>
              <td class="num" class:hi={f.cc > 15}>{f.cc}</td>
              <td class="num">{f.nesting}</td>
              <td class="num muted">{f.start}–{f.end}</td>
              <td class="num">{f.authors}</td>
              <td class="muted">{agoTs(meta.now, f.last_change)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if d.xray.coupling.length}
        <div class="fncouple">
          <h3>Functions that change together</h3>
          {#each d.xray.coupling.slice(0, 8) as c}
            <div class="mono small">{c.a} ↔ {c.b} <span class="muted">· {c.support}× · {pct(c.confidence)}</span></div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  <div class="grid">
    <section class="card pad">
      <h3>Changes together with</h3>
      {#if d.coupling.length === 0}
        <div class="muted">No strong change coupling.</div>
      {:else}
        <table class="data compact">
          <tbody>
            {#each d.coupling as p}
              <tr>
                <td class="nm"><Path path={p.name} repo={p.repo !== d.repo ? p.repo : ''} /></td>
                <td class="barcell"><span class="hbar blue" style="width:{p.confidence * 100}px"></span><span class="num">{pct(p.confidence)}</span></td>
                <td class="num muted">{p.support}×</td>
                <td>
                  {#if p.cross_repo}<span class="pill x">cross-repo</span>{:else if p.cross_unit}<span class="pill">cross-unit</span>{/if}
                  {#if p.by === 'ticket'}<span class="pill">ticket</span>{/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
        <div class="muted small">% = share of this code's changes that also changed the other.</div>
      {/if}
    </section>

    <section class="card pad">
      <h3>Knowledge</h3>
      {#if d.ownership}
        {@const o = d.ownership}
        <div class="dist">
          {#each o.authors as a}<span style="width:{a.share * 100}%;background:{color(a.name)};opacity:{a.active ? 1 : 0.45}" title="{a.name} {pct(a.share)}"></span>{/each}
        </div>
        <ul class="people">
          {#each o.authors.slice(0, 6) as a}
            <li><span class="dot" style="background:{color(a.name)}"></span>{a.name}{#if !a.active}<span class="muted"> (inactive)</span>{/if}<span class="num muted">{pct(a.share)}</span></li>
          {/each}
        </ul>
        <div class="muted small">bus factor {o.bus_factor} · knowledge loss {pct(o.knowledge_loss)} · fragmentation {o.fragmentation.toFixed(2)} · {o.recent_authors} authors in the last year</div>
      {/if}
      {#if d.experts.length}
        <h3 class="sub">Ask</h3>
        <div>{d.experts.map((e: any) => e.name).join(', ')}</div>
      {/if}
    </section>
  </div>

  <section class="card pad">
    <h3>Recent commits {#if d.tickets.length}<span class="muted small">· tickets {d.tickets.slice(0, 8).join(', ')}</span>{/if}</h3>
    <table class="data compact">
      <tbody>
        {#each d.commits.slice(0, 20) as c}
          <tr>
            <td class="mono muted">{c.sha.slice(0, 8)}</td>
            <td class="muted nowrap">{date(c.ts)}</td>
            <td class:fix={c.defect}>{c.subject}</td>
            <td class="muted">{c.author}</td>
            <td class="num muted">+{c.added} −{c.deleted}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header { margin-bottom: 16px; }
  .crumbs { font-size: 12.5px; margin-bottom: 4px; }
  .title { font-size: 20px; overflow-wrap: anywhere; }
  .tags { margin-top: 8px; gap: 6px; }
  .small { font-size: 12px; }
  .stats { display: grid; grid-template-columns: repeat(auto-fill, minmax(165px, 1fr)); gap: 12px; margin-bottom: 14px; }
  .stat { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px; }
  .stat .l { font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); }
  .stat .v { font-size: 22px; font-weight: 600; margin: 2px 0; }
  .reasons ul { margin: 6px 0 0; padding-left: 18px; color: var(--ink-2); }
  .reasons { margin-bottom: 14px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(420px, 1fr)); gap: 14px; margin-bottom: 14px; }
  section.card { margin-bottom: 14px; }
  .grid section.card { margin-bottom: 0; }
  h3 { margin-bottom: 10px; }
  .sub { margin-top: 16px; }
  .compact td { padding: 5px 8px; }
  .ng td { border-bottom: 1px solid var(--grid); font-size: 12px; padding-top: 0; }
  .xray { margin-top: 4px; }
  .fn { max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .barcell { white-space: nowrap; }
  .hbar { display: inline-block; height: 9px; border-radius: 0 4px 4px 0; vertical-align: middle; margin-right: 6px; min-width: 2px; }
  .hbar.blue { background: var(--link); }
  .hi { color: var(--critical); font-weight: 600; }
  .fncouple { margin-top: 14px; }
  .nm { max-width: 320px; }
  .x { border-color: var(--accent); color: var(--accent-ink); }
  .dist { display: flex; gap: 2px; height: 12px; border-radius: 3px; overflow: hidden; background: var(--grid); }
  .dist span { display: block; }
  .people { list-style: none; padding: 0; margin: 10px 0; display: grid; gap: 4px; }
  .people li { display: flex; align-items: center; gap: 8px; }
  .people .num { margin-left: auto; }
  .fix { color: var(--critical); }
  .nowrap { white-space: nowrap; }
</style>

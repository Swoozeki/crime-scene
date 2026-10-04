<script lang="ts">
  // Units (apps, libs, micro-frontends, features, services) as nodes; edges are change coupling
  // between them. Repos are pulled into separate clusters; cross-repo edges are dashed.
  import * as d3 from 'd3';
  import { api } from '../lib/api';
  import { go } from '../lib/router.svelte';
  import { filters, showTip, hideTip } from '../lib/state.svelte';
  import { heat } from '../lib/colors';
  import { num, pct } from '../lib/fmt';
  import Health from '../components/Health.svelte';

  const data = api<any>('architecture');
  let host = $state<HTMLDivElement>();
  let width = $state(900);
  let minSupport = $state(5);
  let showTable = $state(false);

  function scoped(d: any) {
    const units = d.units.filter((u: any) => !filters.repo || u.repo === filters.repo);
    const keys = new Set(units.map((u: any) => u.key));
    const edges = d.edges.filter((e: any) => keys.has(e.a) && keys.has(e.b) && e.support >= minSupport);
    return { units, edges };
  }

  function draw(d: any) {
    if (!host) return;
    const { units, edges } = scoped(d);
    const w = Math.max(400, width);
    const h = Math.max(520, Math.min(900, 300 + units.length * 4));
    host.innerHTML = '';
    if (!units.length) return;
    const repos = [...new Set(units.map((u: any) => u.repo))] as string[];
    const cx = d3.scalePoint<string>().domain(repos).range([w * 0.25, w * 0.75]).padding(repos.length === 1 ? 1 : 0.2);
    const r = d3.scaleSqrt().domain([0, d3.max(units, (u: any) => u.loc as number) ?? 1]).range([4, 34]);
    const wE = d3.scaleLinear().domain([0, d3.max(edges, (e: any) => e.support as number) ?? 1]).range([1, 7]);
    const nodes = units.map((u: any) => ({ ...u, id: u.key }));
    const links = edges.map((e: any) => ({ ...e, source: e.a, target: e.b }));

    const svg = d3.select(host).append('svg').attr('viewBox', `0 0 ${w} ${h}`).attr('width', '100%').attr('role', 'img').attr('aria-label', 'Architecture graph of units and their change coupling');
    const link = svg
      .append('g')
      .selectAll('line')
      .data(links)
      .join('line')
      .attr('stroke', (e: any) => (e.cross_repo ? 'var(--accent)' : 'var(--axis)'))
      .attr('stroke-opacity', (e: any) => 0.35 + 0.65 * Math.max(e.conf_ab, e.conf_ba))
      .attr('stroke-width', (e: any) => wE(e.support))
      .attr('stroke-dasharray', (e: any) => (e.cross_repo ? '6 4' : null))
      .on('mousemove', (ev: MouseEvent, e: any) =>
        showTip(ev, `${e.source.label} ↔ ${e.target.label}`, [
          `changed together ${e.support}× (${e.by})`,
          `${pct(e.conf_ab)} of ${e.source.name}'s changes · ${pct(e.conf_ba)} of ${e.target.name}'s`,
          `lift ${e.lift.toFixed(1)}${e.cross_repo ? ' · crosses repositories' : ''}`,
        ]),
      )
      .on('mouseleave', hideTip);

    const node = svg
      .append('g')
      .selectAll('g')
      .data(nodes)
      .join('g')
      .style('cursor', 'pointer')
      .on('click', (_: MouseEvent, u: any) => {
        hideTip();
        go('hotspots', undefined, { level: 'entity', unit: repos.length > 1 || d.units.some((x: any) => x.repo !== u.repo) ? `${u.repo}:${u.name}` : u.name });
      })
      .on('mousemove', (ev: MouseEvent, u: any) =>
        showTip(ev, u.label, [
          `${u.kind}${u.tags?.length ? ' · ' + u.tags.join(', ') : ''}`,
          `${num(u.loc)} lines · ${u.files} files · ${u.revisions} changes`,
          `hotspot score ${u.score.toFixed(2)} · health ${u.health.toFixed(1)}`,
          `main dev ${u.main_dev ?? '–'} · bus factor ${u.bus_factor ?? '–'} · knowledge loss ${pct(u.knowledge_loss)}`,
        ]),
      )
      .on('mouseleave', hideTip);
    node
      .append('circle')
      .attr('r', (u: any) => r(u.loc))
      .attr('fill', (u: any) => heat(0.08 + u.score * 0.92))
      .attr('stroke', (u: any) => (u.kind === 'mfe' ? 'var(--ink)' : 'var(--surface)'))
      .attr('stroke-width', (u: any) => (u.kind === 'mfe' ? 2.5 : 2));
    // label only the most significant units (size × heat); the rest have tooltips and the table
    const labelled = new Set(
      [...nodes].sort((a: any, b: any) => b.loc * (0.2 + b.score) - a.loc * (0.2 + a.score)).slice(0, 20).map((n: any) => n.id),
    );
    const short = (n: string) => {
      const parts = n.split('/');
      return parts.length > 2 ? parts.slice(-2).join('/') : n;
    };
    node
      .append('text')
      .text((u: any) => short(u.name))
      .attr('y', (u: any) => r(u.loc) + 12)
      .attr('text-anchor', 'middle')
      .style('font', '11px var(--sans)')
      .style('fill', 'var(--ink-2)')
      .style('paint-order', 'stroke')
      .style('stroke', 'var(--surface)')
      .style('stroke-width', '3px')
      .style('pointer-events', 'none')
      .style('display', (u: any) => (labelled.has(u.id) || units.length < 25 ? null : 'none'));

    const sim = d3
      .forceSimulation(nodes)
      .force('link', d3.forceLink(links).id((n: any) => n.id).distance(90).strength((e: any) => 0.05 + 0.4 * Math.max(e.conf_ab, e.conf_ba)))
      .force('charge', d3.forceManyBody().strength(-140))
      .force('x', d3.forceX((n: any) => cx(n.repo) ?? w / 2).strength(0.08))
      .force('y', d3.forceY(h / 2).strength(0.08))
      .force('collide', d3.forceCollide((n: any) => r(n.loc) + 6))
      .stop();
    for (let i = 0; i < 300; i++) sim.tick();
    const clampX = (x: number) => Math.max(20, Math.min(w - 20, x));
    const clampY = (y: number) => Math.max(20, Math.min(h - 24, y));
    link
      .attr('x1', (e: any) => clampX(e.source.x))
      .attr('y1', (e: any) => clampY(e.source.y))
      .attr('x2', (e: any) => clampX(e.target.x))
      .attr('y2', (e: any) => clampY(e.target.y));
    node.attr('transform', (n: any) => `translate(${clampX(n.x)},${clampY(n.y)})`);
    if (repos.length > 1) {
      svg
        .append('g')
        .selectAll('text')
        .data(repos)
        .join('text')
        .attr('x', (rp) => cx(rp)!)
        .attr('y', 18)
        .attr('text-anchor', 'middle')
        .style('font', '600 12px var(--sans)')
        .style('fill', 'var(--muted)')
        .text((rp) => rp);
    }
  }

  $effect(() => {
    void width;
    void minSupport;
    data.then(draw);
  });
</script>

<header class="row wrap">
  <div class="grow">
    <h1>Architecture</h1>
    <div class="muted">Units detected from Angular/Nx projects, module federation, packages and framework layers. Lines show units that change together; dashed orange lines cross repositories (via shared ticket IDs). Circles: size = lines, color = hotspot score, dark ring = micro-frontend.</div>
  </div>
</header>
<div class="row wrap controls">
  <label class="row">Minimum shared changes <input type="range" min="3" max="40" bind:value={minSupport} /> <span class="num">{minSupport}</span></label>
  <button onclick={() => (showTable = !showTable)}>{showTable ? 'Hide table' : 'Show as table'}</button>
</div>

{#await data}
  <div class="empty">Laying out…</div>
{:then d}
  <div class="card pad"><div bind:this={host} bind:clientWidth={width}></div></div>
  {#if showTable}
    {@const s = scoped(d)}
    {@const byKey = new Map(d.units.map((u: any) => [u.key, u]))}
    <div class="card tables">
      <table class="data">
        <thead><tr><th>Unit</th><th>Kind</th><th class="num">Lines</th><th class="num">Score</th><th>Health</th><th>Main dev</th><th class="num">Bus factor</th></tr></thead>
        <tbody>
          {#each s.units as u}
            <tr><td>{u.label}</td><td>{u.kind}</td><td class="num">{num(u.loc)}</td><td class="num">{u.score.toFixed(2)}</td><td><Health value={u.health} /></td><td>{u.main_dev ?? '–'}</td><td class="num">{u.bus_factor ?? '–'}</td></tr>
          {/each}
        </tbody>
      </table>
      <table class="data">
        <thead><tr><th>A</th><th>B</th><th class="num">Together</th><th class="num">A→B</th><th class="num">B→A</th><th>By</th></tr></thead>
        <tbody>
          {#each s.edges as e}
            <tr><td>{(byKey.get(e.a) as any)?.label}</td><td>{(byKey.get(e.b) as any)?.label}</td><td class="num">{e.support}</td><td class="num">{pct(e.conf_ab)}</td><td class="num">{pct(e.conf_ba)}</td><td>{e.by}{e.cross_repo ? ' · cross-repo' : ''}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header, .controls { margin-bottom: 14px; }
  .tables { margin-top: 14px; display: grid; gap: 20px; padding: 8px; }
</style>

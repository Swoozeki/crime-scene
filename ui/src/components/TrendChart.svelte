<script lang="ts">
  // Single-series complexity line with a crosshair tooltip (one series → no legend; the title names it).
  import * as d3 from 'd3';
  import { date } from '../lib/fmt';

  let { points }: { points: { ts: number; complexity: number; loc: number; max_cc: number }[] } = $props();
  let width = $state(600);
  const height = 180;
  const m = { t: 10, r: 14, b: 24, l: 40 };
  let hover = $state<number | null>(null);

  const x = $derived(d3.scaleTime().domain(d3.extent(points, (p) => new Date(p.ts * 1000)) as [Date, Date]).range([m.l, width - m.r]));
  const y = $derived(d3.scaleLinear().domain([0, (d3.max(points, (p) => p.complexity) ?? 1) * 1.1]).nice().range([height - m.b, m.t]));
  const line = $derived(
    d3
      .line<any>()
      .x((p) => x(new Date(p.ts * 1000)))
      .y((p) => y(p.complexity))
      .curve(d3.curveMonotoneX)(points) ?? '',
  );
  const area = $derived(
    d3
      .area<any>()
      .x((p) => x(new Date(p.ts * 1000)))
      .y0(height - m.b)
      .y1((p) => y(p.complexity))
      .curve(d3.curveMonotoneX)(points) ?? '',
  );

  function move(e: MouseEvent) {
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const t = x.invert(e.clientX - rect.left).getTime() / 1000;
    let best = 0;
    for (let i = 1; i < points.length; i++) if (Math.abs(points[i].ts - t) < Math.abs(points[best].ts - t)) best = i;
    hover = best;
  }
</script>

<div bind:clientWidth={width} class="tc">
  {#if points.length < 2}
    <div class="muted">Not enough history to draw a trend.</div>
  {:else}
    <svg {width} {height} role="img" aria-label="Complexity over time" onmousemove={move} onmouseleave={() => (hover = null)}>
      {#each y.ticks(4) as t}
        <line x1={m.l} x2={width - m.r} y1={y(t)} y2={y(t)} stroke="var(--grid)" />
        <text x={m.l - 6} y={y(t)} dy="0.32em" text-anchor="end" class="tick">{t}</text>
      {/each}
      {#each x.ticks(Math.max(2, Math.floor(width / 120))) as t}
        <text x={x(t)} y={height - 6} text-anchor="middle" class="tick">{d3.timeFormat('%b %Y')(t)}</text>
      {/each}
      <line x1={m.l} x2={width - m.r} y1={height - m.b} y2={height - m.b} stroke="var(--axis)" />
      <path d={area} fill="var(--accent)" opacity="0.08" />
      <path d={line} fill="none" stroke="var(--accent)" stroke-width="2" />
      {#if hover !== null}
        {@const p = points[hover]}
        <line x1={x(new Date(p.ts * 1000))} x2={x(new Date(p.ts * 1000))} y1={m.t} y2={height - m.b} stroke="var(--axis)" />
        <circle cx={x(new Date(p.ts * 1000))} cy={y(p.complexity)} r="4.5" fill="var(--accent)" stroke="var(--surface)" stroke-width="2" />
      {/if}
    </svg>
    {#if hover !== null}
      {@const p = points[hover]}
      <div class="readout">
        <b>{date(p.ts)}</b> · complexity {p.complexity.toFixed(0)} · {p.loc} lines · most complex function cc {p.max_cc}
      </div>
    {:else}
      <div class="readout muted">Hover to inspect a version.</div>
    {/if}
  {/if}
</div>

<style>
  .tc { width: 100%; }
  svg { display: block; }
  .tick { font-size: 11px; fill: var(--muted); font-variant-numeric: tabular-nums; }
  .readout { font-size: 12.5px; margin-top: 4px; min-height: 18px; }
</style>

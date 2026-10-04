<script lang="ts">
  // Zoomable circle packing (workspace → repo → unit → dirs → entity). Size = lines of code;
  // color = the chosen measure. Click a group to zoom, a leaf to open it, the background to zoom out.
  import * as d3 from 'd3';
  import { heat, blue, personScale, STATUS, healthStatus } from '../lib/colors';
  import { showTip, hideTip } from '../lib/state.svelte';
  import { ago } from '../lib/fmt';

  type Mode = 'score' | 'health' | 'age' | 'dev' | 'loss';
  let { tree, mode, people = [], onOpen }: { tree: any; mode: Mode; people?: string[]; onOpen: (key: number) => void } = $props();

  let host: HTMLDivElement;
  let width = $state(800);

  function leafColor(d: any, person: (n: string) => string): string {
    const x = d.data;
    switch (mode) {
      case 'score':
        return x.revisions ? heat(0.08 + x.score * 0.92) : heat(0.02);
      case 'health':
        return STATUS[healthStatus(x.health ?? 10)];
      case 'age':
        // recent change = dark; untouched for 3+ years = light
        return x.age_days === undefined ? blue(0) : blue(1 - Math.min(1, x.age_days / 1095));
      case 'dev':
        return person(x.main_dev);
      case 'loss':
        return blue(x.knowledge_loss ?? 0);
    }
  }

  $effect(() => {
    const w = width;
    const m = mode;
    void m;
    const size = Math.max(320, Math.min(w, innerHeight - 230, 1100));
    const root = d3
      .hierarchy(tree)
      .sum((d: any) => (d.children?.length ? 0 : (d.loc ?? 1)))
      .sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
    const pack = d3.pack<any>().size([size, size]).padding((d) => (d.depth === 0 ? 6 : 3));
    const packed = pack(root);
    const person = personScale(people);

    host.innerHTML = '';
    const svg = d3
      .select(host)
      .append('svg')
      .attr('viewBox', `${-size / 2} ${-size / 2} ${size} ${size}`)
      .attr('width', size)
      .attr('height', size)
      .attr('role', 'img')
      .attr('aria-label', 'Hotspot map: circles sized by lines of code')
      .style('display', 'block')
      .style('cursor', 'zoom-out');

    let focus = packed;
    let view: [number, number, number] = [packed.x, packed.y, packed.r * 2];

    const node = svg
      .append('g')
      .selectAll('circle')
      .data(packed.descendants())
      .join('circle')
      .attr('fill', (d) => (d.children ? (d.depth % 2 ? 'var(--surface-2)' : 'var(--surface)') : leafColor(d, person)))
      .attr('stroke', (d) => (d.children ? 'var(--axis)' : 'var(--surface)'))
      .attr('stroke-width', (d) => (d.children ? 0.75 : 1))
      .style('cursor', (d) => (d.children ? 'zoom-in' : 'pointer'))
      .on('mousemove', (e: MouseEvent, d) => {
        const x = d.data;
        if (d.children) {
          showTip(e, x.name, [`${d.leaves().length} entities · ${d3.format(',')(d.value ?? 0)} lines`]);
        } else {
          showTip(e, x.name, [
            `hotspot score ${(x.score ?? 0).toFixed(2)} · ${x.revisions ?? 0} changes`,
            `${d3.format(',')(x.loc ?? 0)} lines · health ${(x.health ?? 10).toFixed(1)}`,
            `main dev ${x.main_dev ?? '–'} · last change ${ago(x.age_days)}`,
            ...(x.knowledge_loss ? [`knowledge loss ${Math.round(x.knowledge_loss * 100)}%`] : []),
          ]);
        }
      })
      .on('mouseleave', hideTip)
      .on('click', (e: MouseEvent, d) => {
        e.stopPropagation();
        if (!d.children) {
          hideTip();
          onOpen(d.data.key);
        } else if (focus !== d) zoom(d);
      });

    const label = svg
      .append('g')
      .attr('pointer-events', 'none')
      .attr('text-anchor', 'middle')
      .style('font', '11px var(--sans)')
      .selectAll('text')
      .data(packed.descendants())
      .join('text')
      .style('fill', 'var(--ink)')
      .style('paint-order', 'stroke')
      .style('stroke', 'var(--surface)')
      .style('stroke-width', '3px')
      .style('font-weight', (d) => (d.children ? 600 : 400))
      .text((d) => d.data.name);

    svg.on('click', () => focus.parent && zoom(focus.parent));

    function zoomTo(v: [number, number, number]) {
      const k = size / v[2];
      view = v;
      // groups are labelled along their top edge so the label doesn't sit on their children
      label.attr('transform', (d) => `translate(${(d.x - v[0]) * k},${(d.y - v[1]) * k + (d.children ? -d.r * k + 14 : 4)})`);
      node.attr('transform', (d) => `translate(${(d.x - v[0]) * k},${(d.y - v[1]) * k})`).attr('r', (d) => d.r * k);
      // label the focus' children when they are big enough to hold text
      label.style('display', (d) => (d.parent === focus && d.r * k > 24 ? 'inline' : 'none'));
    }

    function zoom(d: d3.HierarchyCircularNode<any>) {
      focus = d;
      const t = svg
        .transition()
        .duration(550)
        .tween('zoom', () => {
          const i = d3.interpolateZoom(view, [focus.x, focus.y, focus.r * 2]);
          return (t: number) => zoomTo(i(t));
        });
      void t;
    }

    zoomTo([packed.x, packed.y, packed.r * 2]);
  });
</script>

<div class="cp" bind:this={host} bind:clientWidth={width}></div>

<style>
  .cp { width: 100%; display: flex; justify-content: center; }
</style>

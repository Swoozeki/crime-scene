<script lang="ts">
  import { api } from '../lib/api';
  import { go, route, setQuery } from '../lib/router.svelte';
  import { filters } from '../lib/state.svelte';
  import CirclePack from '../components/CirclePack.svelte';
  import Legend from '../components/Legend.svelte';
  import { topPeople } from '../lib/colors';

  type Mode = 'score' | 'health' | 'age' | 'dev' | 'loss';
  let { initialMode = 'score', title = 'Hotspot map' }: { initialMode?: Mode; title?: string } = $props();
  // svelte-ignore state_referenced_locally
  let mode = $state<Mode>((route.query.color as Mode) ?? initialMode);
  const tree = api<any>('tree');

  function scoped(t: any) {
    if (!filters.repo) return t;
    return { ...t, children: t.children.filter((c: any) => c.name === filters.repo) };
  }

</script>

<header class="row wrap">
  <div class="grow">
    <h1>{title}</h1>
    <div class="muted">Each circle is a component or file, sized by lines of code. Click a group to zoom in, a circle to investigate it, the background to zoom out.</div>
  </div>
</header>

<div class="row wrap controls">
  <div class="seg" role="group" aria-label="Color by">
    {#each [['score', 'Hotspot score'], ['health', 'Code health'], ['age', 'Last change'], ['dev', 'Main developer'], ['loss', 'Knowledge loss']] as [m, label]}
      <button class:on={mode === m} onclick={() => { mode = m as Mode; setQuery({ color: m }); }}>{label}</button>
    {/each}
  </div>
</div>

{#await tree}
  <div class="empty">Packing…</div>
{:then t}
  {@const data = scoped(t)}
  {@const people = topPeople(data)}
  <div class="card pad">
    <Legend {mode} {people} />
    <CirclePack tree={data} {mode} {people} onOpen={(k) => go('entity', k, { level: 'entity' })} />
  </div>
{:catch e}
  <div class="error">{e.message}</div>
{/await}

<style>
  header, .controls { margin-bottom: 14px; }
</style>

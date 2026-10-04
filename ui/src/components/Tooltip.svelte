<script lang="ts">
  import { tip } from '../lib/state.svelte';
  let w = $state(0);
  let h = $state(0);
  const left = $derived(tip.x + 14 + w > innerWidth ? tip.x - w - 14 : tip.x + 14);
  const top = $derived(tip.y + 14 + h > innerHeight ? tip.y - h - 10 : tip.y + 14);
</script>

{#if tip.show}
  <div class="tooltip" style="left:{left}px;top:{top}px" bind:clientWidth={w} bind:clientHeight={h}>
    <div class="t">{tip.title}</div>
    {#each tip.lines as l}<div class="ink2">{l}</div>{/each}
  </div>
{/if}

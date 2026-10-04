<script lang="ts">
  import { STATUS, healthLabel, healthStatus } from '../lib/colors';
  let { value, label = false }: { value: number | null | undefined; label?: boolean } = $props();
  const s = $derived(healthStatus(value ?? 10));
</script>

{#if value === null || value === undefined}
  <span class="muted">–</span>
{:else}
  <span class="h" title="code health {value.toFixed(1)}/10 — {healthLabel[s]}">
    <span class="dot" style="background:{STATUS[s]}"></span>
    <span class="num">{value.toFixed(1)}</span>
    {#if label}<span class="muted">{healthLabel[s]}</span>{/if}
  </span>
{/if}

<style>
  .h { display: inline-flex; align-items: center; gap: 6px; }
</style>

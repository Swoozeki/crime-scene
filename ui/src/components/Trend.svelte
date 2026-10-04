<script lang="ts">
  let { trend }: { trend: { direction: string; change: number } | null | undefined } = $props();
</script>

{#if trend}
  <span class="pill" title="complexity change over the last year: {(trend.change * 100).toFixed(0)}%">
    {#if trend.direction === 'deteriorating'}<span style="color:var(--critical)">↗</span> worse
    {:else if trend.direction === 'improving'}<span style="color:var(--good)">↘</span> better
    {:else}→ stable{/if}
  </span>
{/if}

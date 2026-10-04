<script lang="ts">
  import { heat } from '../lib/colors';
  let { value, width = 64 }: { value: number; width?: number } = $props();
</script>

<span class="sb" style="width:{width}px" title="hotspot score {value.toFixed(2)}">
  <span class="fill" style="width:{Math.max(2, value * width)}px;background:{heat(0.25 + value * 0.75)}"></span>
</span>
<span class="num v">{value.toFixed(2)}</span>

<style>
  .sb { display: inline-block; height: 8px; background: var(--grid); border-radius: 4px; vertical-align: middle; overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 4px; }
  .v { margin-left: 6px; color: var(--ink-2); font-size: 12px; }
</style>

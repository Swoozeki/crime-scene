<script lang="ts">
  import { base, dir } from '../lib/fmt';
  let { path, repo = '' }: { path: string; repo?: string } = $props();
</script>

<span class="p mono" title={(repo ? repo + ':' : '') + path}>
  {#if repo}<span class="muted">{repo}:</span>{/if}{#if dir(path)}<span class="muted">{dir(path)}/</span>{/if}<b>{base(path)}</b>
</span>

<style>
  .p { display: inline-block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; vertical-align: bottom; direction: ltr; }
  b { font-weight: 600; }
</style>

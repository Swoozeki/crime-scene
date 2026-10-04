<script lang="ts">
  import { heat, blue, personScale, STATUS, healthLabel } from '../lib/colors';
  let { mode, people = [] }: { mode: string; people?: string[] } = $props();
  const steps = [0, 0.25, 0.5, 0.75, 1];
  const person = $derived(personScale(people));
</script>

<div class="legend">
  {#if mode === 'score'}
    <span class="muted">cold</span>
    <span class="ramp">{#each steps as s}<span class="sw" style="background:{heat(0.08 + s * 0.92)}"></span>{/each}</span>
    <span class="muted">hot (changes often × complex)</span>
  {:else if mode === 'health'}
    {#each [['good', '≥ 8'], ['warning', '6–8'], ['serious', '4–6'], ['critical', '< 4']] as [s, r]}
      <span class="item"><span class="dot" style="background:{STATUS[s as keyof typeof STATUS]}"></span>{healthLabel[s as keyof typeof healthLabel]} <span class="muted">{r}</span></span>
    {/each}
  {:else if mode === 'age'}
    <span class="muted">3+ years</span>
    <span class="ramp">{#each steps as s}<span class="sw" style="background:{blue(s)}"></span>{/each}</span>
    <span class="muted">changed recently</span>
  {:else if mode === 'loss'}
    <span class="muted">0%</span>
    <span class="ramp">{#each steps as s}<span class="sw" style="background:{blue(s)}"></span>{/each}</span>
    <span class="muted">100% written by inactive authors</span>
  {:else if mode === 'dev'}
    {#each people.slice(0, 7) as p}<span class="item"><span class="dot" style="background:{person(p)}"></span>{p}</span>{/each}
    {#if people.length > 7}<span class="item"><span class="dot" style="background:{person(null)}"></span>Other ({people.length - 7})</span>{/if}
  {/if}
</div>

<style>
  .legend { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; font-size: 12.5px; margin-bottom: 10px; color: var(--ink-2); }
  .ramp { display: inline-flex; gap: 2px; }
  .sw { width: 20px; height: 10px; }
  .sw:first-child { border-radius: 3px 0 0 3px; }
  .sw:last-child { border-radius: 0 3px 3px 0; }
  .item { display: inline-flex; align-items: center; gap: 6px; }
</style>

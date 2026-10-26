<script lang="ts">
  import type { TypeStat } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    stats: TypeStat[];
  }

  let { stats }: Props = $props();

  const total = $derived(stats.reduce((sum, s) => sum + s.size, 0));
</script>

<section class="card" aria-labelledby="types-title">
  <h2 id="types-title">类型分布</h2>
  {#if stats.length === 0}
    <p class="muted">没有文件</p>
  {/if}
  <ul>
    {#each stats as s (s.key)}
      <li>
        <div class="line">
          <span class="swatch" style:background={CATEGORIES[s.key].color}></span>
          <span class="label">{s.label}</span>
          <span class="num">{formatSize(s.size)}</span>
          <span class="num muted pct">{formatPercent(s.size, total)}</span>
        </div>
        <div class="bar" aria-hidden="true">
          <span
            style:width="{total > 0 ? (s.size / total) * 100 : 0}%"
            style:background={CATEGORIES[s.key].color}
          ></span>
        </div>
        <div class="files muted">{s.files.toLocaleString()} 个文件</div>
      </li>
    {/each}
  </ul>
</section>

<style>
  .card {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 12px 14px;
  }

  h2 {
    margin: 0 0 10px;
    font-size: 14px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    flex: none;
  }

  .label {
    flex: 1;
  }

  .num {
    font-variant-numeric: tabular-nums;
  }

  .pct {
    width: 48px;
    text-align: right;
  }

  .muted {
    color: var(--ink-2);
  }

  .bar {
    height: 4px;
    margin-top: 4px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
  }

  .files {
    margin-top: 2px;
    font-size: 12px;
  }
</style>

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
  <h2 id="types-title">存储空间</h2>
  <p class="total">{formatSize(total)}</p>

  <!-- 像 macOS“存储空间”那样的分段胶囊条，每段宽度与该类型的大小成正比。 -->
  <div class="capsule" aria-hidden="true">
    {#each stats as s (s.key)}
      <span
        style:flex-grow={s.size}
        style:background={CATEGORIES[s.key].color}
        title="{s.label} {formatSize(s.size)}"
      ></span>
    {/each}
  </div>

  {#if stats.length === 0}
    <p class="muted">没有文件</p>
  {/if}
  <ul>
    {#each stats as s (s.key)}
      <li>
        <span class="dot" style:background={CATEGORIES[s.key].color}></span>
        <span class="label">
          {s.label}
          <span class="files">{s.files.toLocaleString()} 个文件</span>
        </span>
        <span class="size">
          {formatSize(s.size)}
          <span class="pct">{formatPercent(s.size, total)}</span>
        </span>
      </li>
    {/each}
  </ul>
</section>

<style>
  .card {
    padding: 16px;
    border-radius: var(--radius);
    background: var(--panel);
    box-shadow: var(--shadow);
  }

  h2 {
    margin: 0;
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .total {
    margin: 2px 0 12px;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.01em;
    font-variant-numeric: tabular-nums;
  }

  .capsule {
    display: flex;
    gap: 2px;
    height: 14px;
    margin-bottom: 14px;
    border-radius: 999px;
    overflow: hidden;
    background: var(--track);
  }

  .capsule span {
    flex-basis: 0;
    min-width: 3px;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }

  .dot {
    flex: none;
    width: 9px;
    height: 9px;
    margin-top: 4px;
    border-radius: 50%;
  }

  .label {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .files,
  .pct,
  .muted {
    color: var(--ink-2);
    font-size: 11px;
  }

  .size {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    font-variant-numeric: tabular-nums;
  }
</style>

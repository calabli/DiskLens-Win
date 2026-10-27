<script lang="ts">
  import type { BucketStat, BucketStats, TypeStat } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    stats: TypeStat[];
    buckets: BucketStats | null;
    /** 当前筛选，用来标出点中的那一行。 */
    filter: string;
    /** 按这个条件筛选。`deep` 为 true 时在所有子文件夹中查找并列出结果。 */
    onfilter: (query: string, deep: boolean) => void;
  }

  let { stats, buckets, filter, onfilter }: Props = $props();

  const total = $derived(stats.reduce((sum, s) => sum + s.size, 0));
  const bucketTotal = $derived(buckets?.size.reduce((sum, b) => sum + b.size, 0) ?? 0);
</script>

{#snippet bucketList(title: string, rows: BucketStat[])}
  <h3>{title}</h3>
  <ul class="buckets">
    {#each rows as b (b.label)}
      <li>
        <button
          type="button"
          class="row"
          class:on={b.query !== null && filter === b.query}
          disabled={!b.query}
          title={b.query ? "列出这些文件" : ""}
          onclick={() => b.query && onfilter(b.query, true)}
        >
          <span class="label"
            >{b.label}<span class="files">{b.files.toLocaleString()} 个文件</span></span
          >
          <span class="size"
            >{formatSize(b.size)}<span class="pct">{formatPercent(b.size, bucketTotal)}</span></span
          >
          <span class="bar" aria-hidden="true"
            ><span style:width="{bucketTotal > 0 ? (b.size / bucketTotal) * 100 : 0}%"></span></span
          >
        </button>
      </li>
    {/each}
  </ul>
{/snippet}

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
      {@const q = `type:${s.key}`}
      <li>
        <button
          type="button"
          class="type"
          class:on={filter === q}
          title="在当前文件夹中筛选这一类；按回车在所有子文件夹中查找"
          onclick={() => onfilter(filter === q ? "" : q, false)}
        >
          <span class="dot" style:background={CATEGORIES[s.key].color}></span>
          <span class="label">
            {s.label}
            <span class="files">{s.files.toLocaleString()} 个文件</span>
          </span>
          <span class="size">
            {formatSize(s.size)}
            <span class="pct">{formatPercent(s.size, total)}</span>
          </span>
        </button>
      </li>
    {/each}
  </ul>
</section>

{#if buckets && (buckets.age.length > 0 || buckets.size.length > 0)}
  <section class="card more" aria-label="按时间和大小">
    {@render bucketList("按修改时间", buckets.age)}
    {@render bucketList("按大小", buckets.size)}
  </section>
{/if}

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

  .type {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    margin: -4px -6px;
    padding: 4px 6px;
    border: 0;
    border-radius: 8px;
    background: none;
    box-sizing: content-box;
    text-align: left;
  }

  .type:hover,
  .row:hover:not(:disabled) {
    background: var(--hover);
  }

  .type.on,
  .row.on {
    background: var(--accent-soft);
  }

  .more {
    margin-top: 12px;
  }

  h3 {
    margin: 0 0 8px;
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
  }

  h3:not(:first-child) {
    margin-top: 14px;
  }

  .buckets {
    gap: 4px;
  }

  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 8px;
    width: 100%;
    padding: 5px 6px;
    border: 0;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  .row:disabled {
    cursor: default;
  }

  .row .bar {
    grid-column: 1 / -1;
    height: 4px;
    border-radius: 999px;
    background: var(--track);
    overflow: hidden;
  }

  .row .bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
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

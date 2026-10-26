<script lang="ts">
  import type { SearchHit } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatSize } from "$lib/format";

  interface Props {
    query: string;
    hits: SearchHit[];
    onreveal: (hit: SearchHit) => void;
    onclose: () => void;
  }

  let { query, hits, onreveal, onclose }: Props = $props();
</script>

<div class="results">
  <div class="head">
    <span>“{query}” 的搜索结果：最大的 {hits.length} 项</span>
    <button type="button" onclick={onclose}>返回列表</button>
  </div>
  <ul>
    {#each hits as hit (hit.id)}
      <li>
        <button type="button" class="hit" onclick={() => onreveal(hit)}>
          <span class="swatch" style:background={CATEGORIES[hit.category].color}></span>
          <span class="where">
            <span class="name">{hit.name}{hit.isDir ? "\\" : ""}</span>
            <span class="path">{hit.path}</span>
          </span>
          <span class="num">{formatSize(hit.size)}</span>
        </button>
      </li>
    {:else}
      <li class="empty">没有找到</li>
    {/each}
  </ul>
</div>

<style>
  .results {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    color: var(--ink-2);
    font-size: 12px;
  }

  .head button {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--line-strong);
    border-radius: 8px;
    background: var(--panel);
    font: inherit;
    color: var(--ink);
    cursor: pointer;
  }

  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .hit {
    width: 100%;
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) 80px;
    gap: 8px;
    align-items: center;
    padding: 6px 12px;
    border: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  .hit:hover {
    background: var(--hover);
  }

  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
  }

  .where {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .path {
    color: var(--ink-2);
    font-size: 12px;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .empty {
    padding: 16px;
    color: var(--ink-2);
    text-align: center;
  }
</style>

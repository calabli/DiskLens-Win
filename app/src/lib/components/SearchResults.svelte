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
    <span>“{query}”：最大的 {hits.length} 项</span>
    <button type="button" onclick={onclose}>完成</button>
  </div>
  <ul>
    {#each hits as hit, i (hit.id)}
      <li>
        <button type="button" class="hit" class:odd={i % 2 === 1} onclick={() => onreveal(hit)}>
          <span class="dot" style:background={CATEGORIES[hit.category].color}></span>
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
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px 6px 14px;
    border-bottom: 1px solid var(--line);
    color: var(--ink-2);
    font-size: 12px;
  }

  .head button {
    height: 24px;
    padding: 0 12px;
    border: 0;
    border-radius: 6px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 600;
  }

  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 4px 6px;
    list-style: none;
  }

  .hit {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) 76px;
    gap: 10px;
    align-items: center;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    text-align: left;
  }

  .hit.odd {
    background: var(--stripe);
  }

  .hit:hover {
    background: var(--accent);
    color: var(--accent-ink);
  }

  .hit:hover .path {
    color: inherit;
    opacity: 0.85;
  }

  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
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
    font-size: 11px;
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

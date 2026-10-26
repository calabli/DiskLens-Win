<script lang="ts">
  import type { Entry } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    entries: Entry[];
    /** 满足筛选条件的总数，可能多于 entries。 */
    total: number;
    /** 当前目录的大小，用来算占比。 */
    parentSize: number;
    hoverId: number | null;
    selectedId: number | null;
    onhover: (id: number | null) => void;
    onopen: (id: number) => void;
    onselect: (id: number) => void;
  }

  let { entries, total, parentSize, hoverId, selectedId, onhover, onopen, onselect }: Props =
    $props();

  const ROW = 32;
  /** 可见区域上下多渲染的行数，滚动时不闪白。 */
  const OVERSCAN = 8;

  let viewport: HTMLDivElement;
  let scrollTop = $state(0);
  let viewHeight = $state(0);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(
    Math.min(entries.length, Math.ceil((scrollTop + viewHeight) / ROW) + OVERSCAN),
  );
  const largest = $derived(entries[0]?.size || 1);

  // 换目录时回到顶部。
  $effect(() => {
    void entries;
    if (viewport) viewport.scrollTop = 0;
  });

  // 树图里悬停或选中的项滚到可见范围内。
  $effect(() => {
    const id = selectedId;
    const i = entries.findIndex((e) => e.id === id);
    if (i < 0 || !viewport) return;
    const top = i * ROW;
    if (top < viewport.scrollTop || top + ROW > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = top - viewport.clientHeight / 2;
    }
  });

  function activate(entry: Entry) {
    if (entry.isDir && entry.hasChildren) onopen(entry.id);
    else onselect(entry.id);
  }
</script>

<div class="list">
  <div class="head" aria-hidden="true">
    <span></span><span>名称</span><span class="num">大小</span><span></span><span class="num"
      >占比</span
    ><span class="num">文件数</span>
  </div>
  <div
    class="viewport"
    bind:this={viewport}
    bind:clientHeight={viewHeight}
    onscroll={() => (scrollTop = viewport.scrollTop)}
    onmouseleave={() => onhover(null)}
    role="presentation"
  >
    <div class="spacer" style:height="{entries.length * ROW}px">
      {#each entries.slice(first, last) as entry, i (entry.id)}
        <button
          type="button"
          class="row"
          class:hover={entry.id === hoverId}
          class:selected={entry.id === selectedId}
          style:top="{(first + i) * ROW}px"
          aria-label="{entry.isDir ? '文件夹' : '文件'} {entry.name}，{formatSize(entry.size)}"
          onmouseenter={() => onhover(entry.id)}
          onclick={() => activate(entry)}
        >
          <span class="swatch" style:background={CATEGORIES[entry.category].color}></span>
          <span class="name">
            <span class="text">{entry.name}</span>
            {#if entry.isDir && entry.hasChildren}<span class="chevron" aria-hidden="true">›</span
              >{/if}
          </span>
          <span class="num">{formatSize(entry.size)}</span>
          <span class="bar" aria-hidden="true"
            ><span
              style:width="{Math.max(2, (entry.size / largest) * 100)}%"
              style:background={CATEGORIES[entry.category].color}
            ></span></span
          >
          <span class="num muted">{formatPercent(entry.size, parentSize)}</span>
          <span class="num muted">{entry.isDir ? entry.files.toLocaleString() : ""}</span>
        </button>
      {/each}
    </div>
    {#if entries.length === 0}
      <p class="empty">没有匹配的项目</p>
    {/if}
  </div>
  {#if total > entries.length}
    <p class="more">
      共 {total.toLocaleString()} 项，只列出最大的 {entries.length.toLocaleString()} 项
    </p>
  {/if}
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }

  .head,
  .row {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) 76px 56px 52px 64px;
    gap: 8px;
    align-items: center;
    padding: 0 12px;
  }

  .head {
    height: 30px;
    flex: none;
    border-bottom: 1px solid var(--line);
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 600;
  }

  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    position: relative;
  }

  .spacer {
    position: relative;
  }

  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 32px;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font: inherit;
    color: inherit;
  }

  .row.hover {
    background: var(--hover);
  }

  .row.selected {
    background: var(--selected);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
  }

  .name {
    display: flex;
    gap: 4px;
    min-width: 0;
  }

  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chevron {
    color: var(--ink-3);
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .muted {
    color: var(--ink-2);
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
  }

  .empty,
  .more {
    margin: 0;
    padding: 12px;
    color: var(--ink-2);
    text-align: center;
  }

  .more {
    flex: none;
    border-top: 1px solid var(--line);
    font-size: 12px;
  }
</style>

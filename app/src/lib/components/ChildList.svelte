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

  const ROW = 30;
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

  // 选中的项滚到可见范围内（例如从树图点选、返回上一级后）。
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
    ><span class="num">项目</span>
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
        {@const index = first + i}
        {@const color = CATEGORIES[entry.category].color}
        <button
          type="button"
          class="row"
          class:odd={index % 2 === 1}
          class:hover={entry.id === hoverId}
          class:selected={entry.id === selectedId}
          style:top="{index * ROW}px"
          aria-label="{entry.isDir ? '文件夹' : '文件'} {entry.name}，{formatSize(entry.size)}"
          onmouseenter={() => onhover(entry.id)}
          onclick={() => activate(entry)}
        >
          <span class="icon" aria-hidden="true">
            {#if entry.isDir}
              <svg viewBox="0 0 20 16" width="18" height="15">
                <path
                  d="M1 3a2 2 0 0 1 2-2h4.2l2 2H17a2 2 0 0 1 2 2v1H1z"
                  fill={color}
                  opacity="0.7"
                ></path>
                <path d="M1 5h18v8a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2z" fill={color}></path>
              </svg>
            {:else}
              <svg viewBox="0 0 16 20" width="13" height="16">
                <path
                  d="M2 1h8l5 5v12a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1z"
                  fill="var(--panel)"
                  stroke="var(--ink-3)"
                ></path>
                <path d="M10 1v5h5" fill="none" stroke="var(--ink-3)"></path>
                <rect x="3.5" y="12" width="9" height="4" rx="1" fill={color}></rect>
              </svg>
            {/if}
          </span>
          <span class="name">
            <span class="text">{entry.name}</span>
          </span>
          <span class="num">{formatSize(entry.size)}</span>
          <span class="bar" aria-hidden="true"
            ><span
              style:width="{Math.max(3, (entry.size / largest) * 100)}%"
              style:background={color}
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
    grid-template-columns: 20px minmax(0, 1fr) 72px 52px 50px 64px;
    gap: 8px;
    align-items: center;
    padding: 0 12px;
  }

  .head {
    height: 28px;
    flex: none;
    border-bottom: 1px solid var(--line);
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
  }

  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    position: relative;
    padding: 4px 0;
  }

  .spacer {
    position: relative;
  }

  .row {
    position: absolute;
    left: 6px;
    right: 6px;
    height: 30px;
    padding: 0 6px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    text-align: left;
    cursor: default;
    transition: background-color 0.12s;
  }

  .row.odd {
    background: var(--stripe);
  }

  .row.hover {
    background: var(--hover);
  }

  .row.selected {
    background: var(--accent);
    color: var(--accent-ink);
  }

  .row.selected .muted {
    color: inherit;
    opacity: 0.85;
  }

  .row.selected .bar {
    background: rgb(255 255 255 / 0.3);
  }

  .row.selected .bar span {
    background: var(--accent-ink) !important;
  }

  .row:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .icon {
    display: grid;
    place-items: center;
  }

  .name {
    display: flex;
    min-width: 0;
  }

  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    height: 5px;
    border-radius: 999px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
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
    font-size: 11px;
  }
</style>

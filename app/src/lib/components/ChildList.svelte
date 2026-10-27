<script lang="ts" module>
  /** 可选的列。名称列总是显示。 */
  export type ColumnKey =
    "size" | "allocated" | "percent" | "files" | "modified" | "accessed" | "owner";
  export type SortKey = "name" | "size" | "allocated" | "files" | "modified" | "accessed";

  export const COLUMNS: { key: ColumnKey; label: string; width: string; sort?: SortKey }[] = [
    { key: "size", label: "大小", width: "72px", sort: "size" },
    { key: "allocated", label: "占用", width: "72px", sort: "allocated" },
    { key: "percent", label: "占比", width: "108px", sort: "size" },
    { key: "files", label: "项目", width: "64px", sort: "files" },
    { key: "modified", label: "修改时间", width: "84px", sort: "modified" },
    { key: "accessed", label: "访问时间", width: "84px", sort: "accessed" },
    { key: "owner", label: "所有者", width: "120px" },
  ];

  export const DEFAULT_COLUMNS: ColumnKey[] = ["size", "percent", "files"];
</script>

<script lang="ts">
  import type { Entry, SelectMode } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatDate, formatPercent, formatSize } from "$lib/format";

  interface Props {
    entries: Entry[];
    /** 满足筛选条件的总数，可能多于 entries。 */
    total: number;
    /** 当前目录的大小，用来算占比。 */
    parentSize: number;
    hoverId: number | null;
    /** 选中的项目（可多选）。 */
    selected: Set<number>;
    /** 最近选中的一项，保持在可见范围内。 */
    focusId: number | null;
    /** 显示的可选列，按 COLUMNS 的顺序。 */
    columns: ColumnKey[];
    sort: { key: SortKey; desc: boolean };
    /** 已经查到的所有者；`null` 表示查不到。 */
    owners: Map<number, string | null>;
    onhover: (id: number | null) => void;
    onopen: (id: number) => void;
    onselect: (id: number, mode: SelectMode) => void;
    onsort: (key: SortKey) => void;
    oncolumns: (columns: ColumnKey[]) => void;
    /** 可见的行变化时通知，用来按需查询所有者。 */
    onvisible: (ids: number[]) => void;
    oncontext: (id: number, event: MouseEvent) => void;
    /** 在文件管理器中打开（鼠标中键或行尾的按钮）。 */
    onexplore: (id: number) => void;
  }

  let {
    entries,
    total,
    parentSize,
    hoverId,
    selected,
    focusId,
    columns,
    sort,
    owners,
    onhover,
    onopen,
    onselect,
    onsort,
    oncolumns,
    onvisible,
    oncontext,
    onexplore,
  }: Props = $props();

  let picking = $state(false);
  const shown = $derived(COLUMNS.filter((c) => columns.includes(c.key)));
  const template = $derived(
    ["20px", "minmax(0, 1fr)", ...shown.map((c) => c.width), "18px"].join(" "),
  );

  function toggleColumn(key: ColumnKey) {
    const next = columns.includes(key) ? columns.filter((k) => k !== key) : [...columns, key];
    oncolumns(COLUMNS.map((c) => c.key).filter((k) => next.includes(k)));
  }

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
  const largest = $derived(entries.reduce((m, e) => Math.max(m, e.size), 0) || 1);

  // 可见的行变化时告诉外层（只在显示所有者列时需要）。
  $effect(() => {
    if (!columns.includes("owner")) return;
    const ids = entries.slice(first, last).map((e) => e.id);
    const timer = setTimeout(() => onvisible(ids), 120);
    return () => clearTimeout(timer);
  });

  // 选中的项滚到可见范围内（例如从树图点选、返回上一级后）。
  $effect(() => {
    const id = focusId;
    const i = entries.findIndex((e) => e.id === id);
    if (i < 0 || !viewport) return;
    const top = i * ROW;
    if (top < viewport.scrollTop || top + ROW > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = top - viewport.clientHeight / 2;
    }
  });

  /** 单击打开文件夹；Ctrl 加选或取消，Shift 连续选择（文件夹也只选不打开）。 */
  function activate(entry: Entry, event: MouseEvent) {
    if (event.shiftKey) onselect(entry.id, "range");
    else if (event.ctrlKey || event.metaKey) onselect(entry.id, "toggle");
    else if (entry.isDir && entry.hasChildren) onopen(entry.id);
    else onselect(entry.id, "only");
  }
</script>

<div class="list">
  <div class="head" style:grid-template-columns={template}>
    <span></span>
    <button type="button" class="sort" onclick={() => onsort("name")}
      >名称{sort.key === "name" ? (sort.desc ? " ↓" : " ↑") : ""}</button
    >
    {#each shown as c (c.key)}
      {#if c.sort}
        <button type="button" class="sort num" onclick={() => c.sort && onsort(c.sort)}
          >{c.label}{sort.key === c.sort && c.key !== "percent"
            ? sort.desc
              ? " ↓"
              : " ↑"
            : ""}</button
        >
      {:else}
        <span class="num">{c.label}</span>
      {/if}
    {/each}
    <span class="pick">
      <button
        type="button"
        class="pick-btn"
        aria-label="选择显示的列"
        title="选择显示的列"
        onclick={() => (picking = !picking)}>⋯</button
      >
      {#if picking}
        <div class="pick-shield" role="presentation" onclick={() => (picking = false)}></div>
        <div class="pick-menu" role="menu">
          {#each COLUMNS as c (c.key)}
            <label
              ><input
                type="checkbox"
                checked={columns.includes(c.key)}
                onchange={() => toggleColumn(c.key)}
              />{c.label}</label
            >
          {/each}
        </div>
      {/if}
    </span>
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
          class:selected={selected.has(entry.id)}
          style:top="{index * ROW}px"
          style:grid-template-columns={template}
          aria-label="{entry.isDir ? '文件夹' : '文件'} {entry.name}，{formatSize(entry.size)}"
          aria-pressed={selected.has(entry.id)}
          onmouseenter={() => onhover(entry.id)}
          onclick={(event) => activate(entry, event)}
          onmousedown={(event) => {
            // 中键：不要触发自动滚动。
            if (event.button === 1) event.preventDefault();
          }}
          onauxclick={(event) => {
            if (event.button !== 1) return;
            event.preventDefault();
            onexplore(entry.id);
          }}
          oncontextmenu={(event) => {
            event.preventDefault();
            oncontext(entry.id, event);
          }}
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
          {#each shown as c (c.key)}
            {#if c.key === "size"}
              <span class="num">{formatSize(entry.size)}</span>
            {:else if c.key === "allocated"}
              <span class="num muted">{formatSize(entry.allocated)}</span>
            {:else if c.key === "percent"}
              <span class="pct"
                ><span class="bar" aria-hidden="true"
                  ><span
                    style:width="{Math.max(3, (entry.size / largest) * 100)}%"
                    style:background={color}
                  ></span></span
                ><span class="num muted">{formatPercent(entry.size, parentSize)}</span></span
              >
            {:else if c.key === "files"}
              <span class="num muted">{entry.isDir ? entry.files.toLocaleString() : ""}</span>
            {:else if c.key === "modified"}
              <span class="num muted">{formatDate(entry.modified)}</span>
            {:else if c.key === "accessed"}
              <span class="num muted">{formatDate(entry.accessed)}</span>
            {:else if c.key === "owner"}
              <span class="muted owner"
                >{owners.get(entry.id) ?? (owners.has(entry.id) ? "—" : "…")}</span
              >
            {/if}
          {/each}
          <!-- 行本身是按钮，里面不能再放按钮；键盘可用 Ctrl+Enter 或右键菜单。 -->
          <span
            class="explore"
            role="presentation"
            title={entry.isDir
              ? "在文件管理器中打开（鼠标中键、Ctrl+Enter）"
              : "在文件管理器中显示（鼠标中键、Ctrl+Enter）"}
            onclick={(event) => {
              event.stopPropagation();
              onexplore(entry.id);
            }}
            ><svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
              ><path
                d="M1.5 4.5a1.5 1.5 0 0 1 1.5-1.5h3l1.5 1.5H13a1.5 1.5 0 0 1 1.5 1.5v5.5A1.5 1.5 0 0 1 13 13H3a1.5 1.5 0 0 1-1.5-1.5z"
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
              ></path><path
                d="M7 9.5h4.5M9.5 7.5l2 2-2 2"
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
                stroke-linecap="round"
              ></path></svg
            ></span
          >
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
    gap: 8px;
    align-items: center;
    padding: 0 12px;
  }

  .sort {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    white-space: nowrap;
  }

  .sort.num {
    text-align: right;
  }

  .sort:hover {
    color: var(--ink);
  }

  .pick {
    position: relative;
  }

  .pick-btn {
    width: 18px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink-2);
  }

  .pick-shield {
    position: fixed;
    inset: 0;
    z-index: 9;
  }

  .pick-menu {
    position: absolute;
    top: 22px;
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 120px;
    padding: 6px 8px;
    border-radius: 9px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    font-weight: 400;
  }

  .pick-menu label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 2px;
    color: var(--ink);
    white-space: nowrap;
  }

  .pct {
    display: grid;
    grid-template-columns: 52px 1fr;
    gap: 6px;
    align-items: center;
  }

  .explore {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    color: var(--ink-2);
    opacity: 0;
    cursor: pointer;
  }

  .row:hover .explore,
  .row.selected .explore {
    opacity: 1;
  }

  .explore:hover {
    background: var(--hover);
    color: var(--accent);
  }

  .row.selected .explore {
    color: inherit;
  }

  .owner {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

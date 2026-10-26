<script lang="ts">
  import {
    listChildren,
    nodeInfo,
    search,
    typeStats,
    type ChildList,
    type NodeInfo,
    type ScanSummary,
    type SearchHit,
    type TypeStat,
  } from "$lib/api";
  import { formatPercent, formatSize } from "$lib/format";
  import ChildListView from "./ChildList.svelte";
  import SearchResults from "./SearchResults.svelte";
  import Treemap from "./Treemap.svelte";
  import TypeStats from "./TypeStats.svelte";

  interface Props {
    summary: ScanSummary;
    onrescan: () => void;
    onhome: () => void;
  }

  let { summary, onrescan, onhome }: Props = $props();

  const SEARCH_LIMIT = 500;

  // 只取初始值是有意的：每次重新扫描，页面会用 {#key} 重建整个 Explorer。
  // svelte-ignore state_referenced_locally
  let dirId = $state(summary.rootId);
  let info = $state<NodeInfo | null>(null);
  let all = $state<ChildList>({ total: 0, entries: [] });
  let filtered = $state<ChildList | null>(null);
  let stats = $state<TypeStat[]>([]);
  let hoverId = $state<number | null>(null);
  let selectedId = $state<number | null>(null);
  let filter = $state("");
  let filterError = $state("");
  let hits = $state<SearchHit[] | null>(null);
  let searchedFor = $state("");
  let error = $state("");

  const entries = $derived(new Map(all.entries.map((e) => [e.id, e])));
  const highlight = $derived(filtered ? new Set(filtered.entries.map((e) => e.id)) : null);
  const focus = $derived(entries.get(hoverId ?? selectedId ?? -1));

  // 进入目录：取节点信息、子项和类型分布。只保留最后一次请求的结果。
  let loadSeq = 0;
  $effect(() => {
    const id = dirId;
    const mine = ++loadSeq;
    Promise.all([nodeInfo(id), listChildren(id, null), typeStats(id)])
      .then(([i, l, s]) => {
        if (mine !== loadSeq) return;
        info = i;
        all = l;
        stats = s;
        error = "";
      })
      .catch((e) => {
        if (mine === loadSeq) error = String(e);
      });
  });

  // 筛选当前目录的直接子项，输入时稍作防抖。
  let filterSeq = 0;
  $effect(() => {
    const id = dirId;
    const text = filter.trim();
    const mine = ++filterSeq;
    if (!text) {
      filtered = null;
      filterError = "";
      return;
    }
    const timer = setTimeout(() => {
      listChildren(id, text)
        .then((l) => {
          if (mine !== filterSeq) return;
          filtered = l;
          filterError = "";
        })
        .catch((e) => {
          if (mine === filterSeq) filterError = String(e);
        });
    }, 200);
    return () => clearTimeout(timer);
  });

  function open(id: number) {
    dirId = id;
    hoverId = null;
    selectedId = null;
    hits = null;
  }

  function up() {
    if (info?.parent != null) {
      const from = dirId;
      open(info.parent);
      selectedId = from;
    }
  }

  async function searchSubtree(event: SubmitEvent) {
    event.preventDefault();
    const text = filter.trim();
    if (!text) return;
    try {
      hits = await search(dirId, text, SEARCH_LIMIT);
      searchedFor = text;
      filterError = "";
    } catch (e) {
      filterError = String(e);
    }
  }

  function reveal(hit: SearchHit) {
    if (hit.parent == null) return;
    open(hit.parent);
    selectedId = hit.id;
    filter = "";
  }

  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (event.key === "Backspace" && target.tagName !== "INPUT") {
      event.preventDefault();
      up();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="explorer">
  <header>
    <button type="button" class="brand" onclick={onhome} title="返回磁盘列表">DiskLens</button>
    <form class="search" onsubmit={searchSubtree} role="search">
      <input
        aria-label="筛选当前目录；按回车在整个子树中搜索"
        placeholder="筛选当前目录，回车搜索全部子目录。例如 size:>1gb ext:iso;zip"
        bind:value={filter}
        spellcheck="false"
        autocomplete="off"
      />
    </form>
    <button type="button" class="secondary" onclick={onrescan}>重新扫描</button>
    <span class="badge" class:good={summary.mode === "mft"}>
      {summary.mode === "mft" ? "MFT 直读" : "普通遍历"}
    </span>
  </header>

  {#if summary.fallbackReason}
    <p class="notice">
      未能使用 MFT 直读（{summary.fallbackReason}），本次用的是普通遍历。以管理员身份运行可以更快、更准确。
    </p>
  {/if}
  {#if filterError}
    <p class="notice" role="alert">{filterError}</p>
  {/if}
  {#if error}
    <p class="notice" role="alert">{error}</p>
  {/if}

  <nav class="crumbs" aria-label="当前位置">
    <button
      type="button"
      class="up"
      onclick={up}
      disabled={info?.parent == null}
      aria-label="返回上一级">↑</button
    >
    {#each info?.crumbs ?? [] as crumb, i (crumb.id)}
      {#if i > 0}<span class="sep" aria-hidden="true">›</span>{/if}
      <button
        type="button"
        class="crumb"
        aria-current={crumb.id === dirId ? "location" : undefined}
        onclick={() => open(crumb.id)}>{crumb.name}</button
      >
    {/each}
    {#if info}
      <span class="here">
        {formatSize(info.size)} · {info.files.toLocaleString()} 个文件 · 占扫描范围 {formatPercent(
          info.size,
          summary.totalSize,
        )}
      </span>
    {/if}
  </nav>

  <div class="body">
    <section class="panel list" aria-label="子项列表">
      {#if hits}
        <SearchResults query={searchedFor} {hits} onreveal={reveal} onclose={() => (hits = null)} />
      {:else}
        <ChildListView
          entries={filtered?.entries ?? all.entries}
          total={filtered?.total ?? all.total}
          parentSize={info?.size ?? 0}
          {hoverId}
          {selectedId}
          onhover={(id) => (hoverId = id)}
          onopen={open}
          onselect={(id) => (selectedId = id)}
        />
      {/if}
    </section>

    <section class="panel map" aria-label="矩形树图">
      <div class="info">
        {#if focus}
          <strong>{focus.name}</strong> · {formatSize(focus.size)} · 占 {formatPercent(
            focus.size,
            info?.size ?? 0,
          )}{focus.isDir ? ` · ${focus.files.toLocaleString()} 个文件` : ""}
        {:else}
          悬停查看详情，点击文件夹进入下一层，Backspace 返回上一级
        {/if}
      </div>
      <Treemap
        {dirId}
        {entries}
        {hoverId}
        {selectedId}
        {highlight}
        onhover={(id) => (hoverId = id)}
        onopen={open}
        onselect={(id) => (selectedId = id)}
      />
    </section>

    <aside class="side">
      <TypeStats {stats} />
    </aside>
  </div>

  <footer>
    <span>{summary.root}</span>
    <span>逻辑大小 {formatSize(summary.totalSize)}</span>
    <span>占用 {formatSize(summary.totalAllocated)}</span>
    <span>{summary.fileCount.toLocaleString()} 个文件</span>
    <span>扫描用时 {(summary.elapsedMs / 1000).toFixed(1)} 秒</span>
    {#if summary.skipped > 0}<span>{summary.skipped} 个目录无权读取</span>{/if}
  </footer>
</div>

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }

  .brand {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    font-size: 16px;
    font-weight: 700;
    color: inherit;
    cursor: pointer;
  }

  .search {
    flex: 1;
    max-width: 560px;
  }

  .search input {
    width: 100%;
    height: 36px;
    box-sizing: border-box;
    padding: 0 12px;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--bg);
    font: inherit;
    color: inherit;
  }

  .secondary {
    height: 36px;
    padding: 0 14px;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--panel);
    font: inherit;
    font-weight: 600;
    color: inherit;
    cursor: pointer;
  }

  .badge {
    margin-left: auto;
    padding: 4px 10px;
    border-radius: 8px;
    background: var(--track);
    font-size: 12px;
    font-weight: 600;
  }

  .badge.good {
    background: var(--good-bg);
    color: var(--good);
  }

  .notice {
    margin: 8px 16px 0;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--warn-bg);
    color: var(--warn);
  }

  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    padding: 8px 16px;
  }

  .up {
    width: 32px;
    height: 32px;
    margin-right: 6px;
    border: 1px solid var(--line-strong);
    border-radius: 8px;
    background: var(--panel);
    font: inherit;
    cursor: pointer;
  }

  .up:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .crumb {
    padding: 4px 6px;
    border: 0;
    border-radius: 6px;
    background: none;
    font: inherit;
    font-size: 14px;
    color: var(--accent);
    cursor: pointer;
  }

  .crumb[aria-current] {
    color: var(--ink);
    font-weight: 600;
  }

  .sep {
    color: var(--ink-3);
  }

  .here {
    margin-left: auto;
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(320px, 420px) minmax(0, 1fr) 260px;
    gap: 12px;
    padding: 0 16px 12px;
  }

  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 12px;
    overflow: hidden;
  }

  .map .info {
    padding: 8px 12px;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .map .info strong {
    color: var(--ink);
  }

  .map :global(.treemap) {
    margin: 0 8px 8px;
  }

  .side {
    min-height: 0;
    overflow-y: auto;
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    padding: 6px 16px;
    background: var(--panel);
    border-top: 1px solid var(--line);
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 1100px) {
    .body {
      grid-template-columns: minmax(300px, 380px) minmax(0, 1fr);
    }

    .side {
      display: none;
    }
  }
</style>

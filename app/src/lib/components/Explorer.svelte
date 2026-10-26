<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
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
  /** 列表滑入的方向：1 进入子目录，-1 返回，0 跳转。 */
  let direction = $state(0);
  let back = $state<number[]>([]);
  let forward = $state<number[]>([]);
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
  const title = $derived(info ? (info.parent == null ? summary.root : info.name) : "");

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

  /** 切换到目录 `id`；`dir` 决定列表滑入方向。 */
  function go(id: number, dir: number, record = true) {
    if (id === dirId) return;
    if (record) {
      back = [...back, dirId];
      forward = [];
    }
    direction = dir;
    dirId = id;
    hoverId = null;
    selectedId = null;
    hits = null;
  }

  function open(id: number) {
    go(id, 1);
  }

  function up() {
    if (info?.parent == null) return;
    const from = dirId;
    go(info.parent, -1);
    selectedId = from;
  }

  function goBack() {
    const prev = back.at(-1);
    if (prev === undefined) return;
    back = back.slice(0, -1);
    forward = [...forward, dirId];
    const from = dirId;
    go(prev, -1, false);
    selectedId = from;
  }

  function goForward() {
    const next = forward.at(-1);
    if (next === undefined) return;
    forward = forward.slice(0, -1);
    back = [...back, dirId];
    go(next, 1, false);
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
    go(hit.parent, 0);
    selectedId = hit.id;
    filter = "";
  }

  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (target.tagName === "INPUT") return;
    if (event.key === "Backspace" || (event.altKey && event.key === "ArrowUp")) {
      event.preventDefault();
      up();
    } else if (event.altKey && event.key === "ArrowLeft") {
      event.preventDefault();
      goBack();
    } else if (event.altKey && event.key === "ArrowRight") {
      event.preventDefault();
      goForward();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="explorer">
  <header class="toolbar">
    <div class="nav" role="group" aria-label="导航">
      <button type="button" onclick={goBack} disabled={back.length === 0} aria-label="后退（Alt+←）"
        ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
          ><path d="M10 3 5 8l5 5" fill="none" stroke="currentColor" stroke-width="1.8"></path></svg
        ></button
      >
      <button
        type="button"
        onclick={goForward}
        disabled={forward.length === 0}
        aria-label="前进（Alt+→）"
        ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
          ><path d="m6 3 5 5-5 5" fill="none" stroke="currentColor" stroke-width="1.8"></path></svg
        ></button
      >
      <button
        type="button"
        onclick={up}
        disabled={info?.parent == null}
        aria-label="上一级（Backspace）"
        ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
          ><path
            d="M8 13V3M3.5 7.5 8 3l4.5 4.5"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
          ></path></svg
        ></button
      >
    </div>

    <div class="title">
      <h1>{title}</h1>
      {#if info}
        <span>{formatSize(info.size)} · {info.files.toLocaleString()} 个文件</span>
      {/if}
    </div>

    <form class="search" onsubmit={searchSubtree} role="search">
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
        ><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.6"
        ></circle><path d="m10.5 10.5 3 3" stroke="currentColor" stroke-width="1.6"></path></svg
      >
      <input
        aria-label="筛选当前文件夹；按回车在所有子文件夹中搜索"
        placeholder="搜索，如 size:>1gb ext:iso"
        bind:value={filter}
        spellcheck="false"
        autocomplete="off"
      />
    </form>

    <span class="mode" class:good={summary.mode === "mft"}>
      {summary.mode === "mft" ? "MFT 直读" : "普通遍历"}
    </span>
    <button type="button" class="tool" onclick={onrescan} aria-label="重新扫描" title="重新扫描"
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><path
          d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M12 1.5v3h-3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
        ></path></svg
      ></button
    >
    <button
      type="button"
      class="tool"
      onclick={onhome}
      aria-label="选择其他磁盘"
      title="选择其他磁盘"
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><rect
          x="2"
          y="4"
          width="12"
          height="8"
          rx="2"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
        ></rect><circle cx="11" cy="8" r="1" fill="currentColor"></circle></svg
      ></button
    >
  </header>

  {#each [summary.fallbackReason ? `未能使用 MFT 直读（${summary.fallbackReason}），本次是普通遍历。以管理员身份运行会更快、更准确。` : "", filterError, error].filter(Boolean) as message (message)}
    <p class="banner" role="status">{message}</p>
  {/each}

  <div class="body">
    <section class="panel list" aria-label="项目列表">
      {#if hits}
        <SearchResults query={searchedFor} {hits} onreveal={reveal} onclose={() => (hits = null)} />
      {:else}
        {#key dirId}
          <div
            class="slide"
            in:fly={{ x: direction * 28, duration: direction === 0 ? 0 : 240, easing: cubicOut }}
          >
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
          </div>
        {/key}
      {/if}
    </section>

    <section class="panel map" aria-label="矩形树图">
      <div class="map-head">
        {#if focus}
          <strong>{focus.name}</strong>
          <span
            >{formatSize(focus.size)} · {formatPercent(focus.size, info?.size ?? 0)}{focus.isDir
              ? ` · ${focus.files.toLocaleString()} 个文件`
              : ""}</span
          >
        {:else}
          <span>点按文件夹放大查看，Backspace 返回上一级</span>
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
    <nav class="path" aria-label="路径">
      {#each info?.crumbs ?? [] as crumb, i (crumb.id)}
        {#if i > 0}<span class="sep" aria-hidden="true">›</span>{/if}
        <button
          type="button"
          aria-current={crumb.id === dirId ? "location" : undefined}
          onclick={() => go(crumb.id, crumb.id === dirId ? 0 : -1)}
          ><svg viewBox="0 0 20 16" width="13" height="11" aria-hidden="true"
            ><path d="M1 3a2 2 0 0 1 2-2h4.2l2 2H17a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2z"
            ></path></svg
          >{i === 0 ? summary.root : crumb.name}</button
        >
      {/each}
    </nav>
    <div class="status">
      {summary.fileCount.toLocaleString()} 个文件，逻辑大小 {formatSize(summary.totalSize)}，占用 {formatSize(
        summary.totalAllocated,
      )}，扫描用时 {(summary.elapsedMs / 1000).toFixed(1)} 秒{summary.skipped > 0
        ? `，${summary.skipped} 个文件夹无权读取`
        : ""}
    </div>
  </footer>
</div>

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 52px;
    padding: 0 14px;
    background: var(--panel-2);
    border-bottom: 1px solid var(--line);
  }

  .nav {
    display: flex;
    gap: 2px;
  }

  .nav button,
  .tool {
    display: grid;
    place-items: center;
    width: 30px;
    height: 28px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--ink-2);
    transition: background-color 0.12s;
  }

  .nav button:hover:not(:disabled),
  .tool:hover {
    background: var(--hover);
    color: var(--ink);
  }

  .nav button:disabled {
    opacity: 0.3;
  }

  .title {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title span {
    color: var(--ink-2);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: min(320px, 32vw);
    height: 28px;
    padding: 0 8px;
    border-radius: 7px;
    background: var(--field);
    color: var(--ink-2);
    transition: box-shadow 0.15s;
  }

  .search:focus-within {
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    outline: none;
    color: var(--ink);
  }

  .search input::placeholder {
    color: var(--ink-3);
  }

  .mode {
    padding: 3px 9px;
    border-radius: 999px;
    background: var(--track);
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }

  .mode.good {
    background: var(--good-bg);
    color: var(--good);
  }

  .banner {
    margin: 10px 14px 0;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--warn-bg);
    color: var(--warn);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(320px, 400px) minmax(0, 1fr) 250px;
    gap: 12px;
    padding: 12px 14px;
  }

  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--panel);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  .slide {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .map-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-height: 18px;
    padding: 10px 14px 8px;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }

  .map-head strong {
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .map :global(.treemap) {
    margin: 0 10px 10px;
  }

  .side {
    min-height: 0;
    overflow-y: auto;
  }

  footer {
    display: flex;
    flex-direction: column;
    background: var(--panel-2);
    border-top: 1px solid var(--line);
    font-size: 11px;
  }

  .path {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 26px;
    padding: 0 10px;
    overflow-x: auto;
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
  }

  .path button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink-2);
  }

  .path button:hover {
    background: var(--hover);
    color: var(--ink);
  }

  .path button[aria-current] {
    color: var(--ink);
    font-weight: 600;
  }

  .path svg {
    fill: var(--cat-app);
  }

  .sep {
    color: var(--ink-3);
  }

  .status {
    padding: 4px 14px 5px;
    color: var(--ink-2);
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 1100px) {
    .body {
      grid-template-columns: minmax(300px, 360px) minmax(0, 1fr);
    }

    .side {
      display: none;
    }
  }
</style>

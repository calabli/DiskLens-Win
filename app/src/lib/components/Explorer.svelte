<script lang="ts">
  import { untrack } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import {
    deleteItems,
    listChildren,
    nodeInfo,
    planDelete,
    refreshScan,
    search,
    typeStats,
    type ChildList,
    type DeletePlan,
    type NodeInfo,
    type ScanSummary,
    type SearchHit,
    type SelectMode,
    type TypeStat,
  } from "$lib/api";
  import { formatAgo, formatPercent, formatSize } from "$lib/format";
  import ChildListView from "./ChildList.svelte";
  import ConfirmDelete from "./ConfirmDelete.svelte";
  import Duplicates from "./Duplicates.svelte";
  import SearchResults from "./SearchResults.svelte";
  import Sunburst from "./Sunburst.svelte";
  import Toast from "./Toast.svelte";
  import Treemap from "./Treemap.svelte";
  import TypeStats from "./TypeStats.svelte";

  interface Props {
    summary: ScanSummary;
    onrescan: () => void;
    onhome: () => void;
  }

  let { summary, onrescan, onhome }: Props = $props();

  const SEARCH_LIMIT = 500;
  const LAYOUT_KEY = "disklens.panels";

  type ChartKind = "treemap" | "sunburst";

  /** 读取上次的侧栏展开状态和图表类型；存储不可用时用默认值。 */
  function loadPanels(): { list: boolean; side: boolean; chart: ChartKind } {
    try {
      const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}");
      return {
        list: saved.list !== false,
        side: saved.side !== false,
        chart: saved.chart === "sunburst" ? "sunburst" : "treemap",
      };
    } catch {
      return { list: true, side: true, chart: "treemap" };
    }
  }

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
  /** 选中的项目，按选中顺序；最后一项是焦点。 */
  let selection = $state<number[]>([]);
  /** Shift 连续选择的起点。 */
  let anchor = $state<number | null>(null);
  let panels = $state(loadPanels());
  let filter = $state("");
  let filterError = $state("");
  let hits = $state<SearchHit[] | null>(null);
  let searchedFor = $state("");
  let error = $state("");
  /** 删除后递增，触发列表、树图、统计重新加载。 */
  let version = $state(0);
  /** 删除之后扫描范围的最新汇总（状态栏用）。 */
  let rootInfo = $state<NodeInfo | null>(null);
  let plan = $state<DeletePlan | null>(null);
  let deleting = $state(false);
  let toast = $state<{ message: string; detail: string[]; tone: "info" | "warn" } | null>(null);
  let refreshing = $state(false);
  /** 正在看重复文件（占据主区域）。 */
  let dupOpen = $state(false);
  /** 本次打开后删除过的项目，用来从重复文件结果里去掉。 */
  let removed = $state(new Set<number>());
  /** 最近一次增量刷新的结果，显示在状态栏。 */
  let refreshNote = $state<string | null>(null);
  let lastRefresh = 0;
  /** 窗口重新获得焦点时，距上次刷新至少这么久才再刷新。 */
  const FOCUS_REFRESH_MS = 5000;

  const entries = $derived(new Map(all.entries.map((e) => [e.id, e])));
  const highlight = $derived(filtered ? new Set(filtered.entries.map((e) => e.id)) : null);
  const visible = $derived(filtered?.entries ?? all.entries);
  const selected = $derived(new Set(selection));
  const selectedId = $derived(selection.at(-1) ?? null);
  const selectedSize = $derived(
    selection.reduce((sum, id) => sum + (entries.get(id)?.size ?? 0), 0),
  );
  /** 悬停在旭日图外圈（不是当前目录的直接子项）时，现查这一项的信息。 */
  let hoverDetail = $state<NodeInfo | null>(null);
  const focus = $derived.by(() => {
    const id = hoverId ?? (selection.length === 1 ? selection[0] : -1);
    return entries.get(id) ?? (hoverDetail?.id === id ? hoverDetail : undefined);
  });

  let detailSeq = 0;
  $effect(() => {
    const id = hoverId;
    const mine = ++detailSeq;
    if (id == null || untrack(() => entries.has(id))) return;
    nodeInfo(id)
      .then((n) => {
        if (mine === detailSeq) hoverDetail = n;
      })
      .catch(() => {});
  });
  const title = $derived(info ? (info.parent == null ? summary.root : info.name) : "");

  // 进入目录：取节点信息、子项和类型分布。只保留最后一次请求的结果。
  let loadSeq = 0;
  $effect(() => {
    const id = dirId;
    void version;
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
        if (mine !== loadSeq) return;
        // 刷新后当前文件夹可能已被删除，回到根目录。
        if (id !== summary.rootId && String(e).includes("已不存在")) {
          back = [];
          forward = [];
          dirId = summary.rootId;
        } else {
          error = String(e);
        }
      });
  });

  // 筛选当前目录的直接子项，输入时稍作防抖。
  let filterSeq = 0;
  $effect(() => {
    const id = dirId;
    const text = filter.trim();
    void version;
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

  $effect(() => {
    const value = JSON.stringify(panels);
    try {
      localStorage.setItem(LAYOUT_KEY, value);
    } catch {
      // 存不下也不影响使用。
    }
  });

  /** 选中 `id`；`only` 只选它，`toggle` 加选或取消，`range` 从起点连续选到它。 */
  function select(id: number, mode: SelectMode) {
    if (mode === "toggle") {
      selection = selection.includes(id) ? selection.filter((x) => x !== id) : [...selection, id];
      anchor = id;
      return;
    }
    const from = visible.findIndex((e) => e.id === anchor);
    const to = visible.findIndex((e) => e.id === id);
    if (mode === "only" || from < 0 || to < 0) {
      selectOnly(id);
      return;
    }
    const range = visible.slice(Math.min(from, to), Math.max(from, to) + 1).map((e) => e.id);
    selection = [...range.filter((x) => x !== id), id];
  }

  function selectOnly(id: number | null) {
    selection = id == null ? [] : [id];
    anchor = id;
  }

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
    selectOnly(null);
    hits = null;
  }

  function open(id: number) {
    go(id, 1);
  }

  function up() {
    if (info?.parent == null) return;
    const from = dirId;
    go(info.parent, -1);
    selectOnly(from);
  }

  function goBack() {
    const prev = back.at(-1);
    if (prev === undefined) return;
    back = back.slice(0, -1);
    forward = [...forward, dirId];
    const from = dirId;
    go(prev, -1, false);
    selectOnly(from);
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
    selectOnly(hit.id);
    filter = "";
  }

  /** 选中项先给出删除汇总，确认后再删。 */
  async function askDelete(ids: number[] = selection) {
    if (ids.length === 0 || plan) return;
    try {
      plan = await planDelete(ids);
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  async function confirmDelete(permanent: boolean) {
    if (!plan) return;
    const ids = plan.items.filter((i) => !i.blocked).map((i) => i.id);
    deleting = true;
    try {
      const result = await deleteItems(ids, permanent);
      const done = result.deleted.length;
      const verb = permanent ? "已彻底删除" : "已移到回收站";
      toast = {
        message: done > 0 ? `${verb}，释放 ${formatSize(result.freed)}` : "没有删除任何项目",
        detail: [
          ...result.failed.map((f) => `${f.path}：${f.reason}`),
          ...(result.logError ? [result.logError] : []),
        ],
        tone: result.failed.length > 0 || result.logError ? "warn" : "info",
      };
      if (done > 0) {
        const gone = new Set(result.deleted);
        removed = new Set([...removed, ...gone]);
        selection = selection.filter((id) => !gone.has(id));
        if (anchor != null && gone.has(anchor)) anchor = null;
        hoverId = null;
        if (hits) hits = hits.filter((h) => !gone.has(h.id));
        rootInfo = await nodeInfo(summary.rootId);
        version++;
      }
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    } finally {
      deleting = false;
      plan = null;
    }
  }

  /**
   * 按 USN 日志增量刷新。`manual` 为 true 时（点按刷新按钮），不能增量刷新就完整重扫；
   * 自动刷新时只在状态栏说明原因。
   */
  async function refresh(manual: boolean) {
    if (refreshing || plan || deleting) return;
    if (summary.mode !== "mft") {
      if (manual) onrescan();
      return;
    }
    refreshing = true;
    lastRefresh = Date.now();
    try {
      const r = await refreshScan();
      if (r.kind === "needsFullScan") {
        // 自动刷新做不到时不打扰，状态栏照常显示扫描时间。
        if (manual) onrescan();
        return;
      }
      const changed = r.added + r.removed + r.updated;
      refreshNote =
        changed > 0
          ? `已刷新，${changed.toLocaleString()} 项有变化（${(r.elapsedMs / 1000).toFixed(1)} 秒）`
          : "已是最新";
      if (changed > 0) {
        selectOnly(null);
        hoverId = null;
        hits = null;
        rootInfo = await nodeInfo(summary.rootId);
        version++;
      }
    } catch (e) {
      if (manual) toast = { message: String(e), detail: [], tone: "warn" };
    } finally {
      refreshing = false;
    }
  }

  // 打开上次的结果后立即刷新一次。只在挂载时运行，不跟踪任何状态。
  $effect(() => {
    untrack(() => {
      if (summary.fromCache) refresh(false);
    });
  });

  function onfocus() {
    if (Date.now() - lastRefresh > FOCUS_REFRESH_MS) refresh(false);
  }

  /** 关闭重复文件，在主界面中定位到 `id`。 */
  async function revealNode(id: number) {
    try {
      const n = await nodeInfo(id);
      dupOpen = false;
      if (n.parent != null) {
        go(n.parent, 0);
        selectOnly(id);
      }
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (["INPUT", "SELECT"].includes(target.tagName) || plan) return;
    if (dupOpen) {
      if (event.key === "Escape") dupOpen = false;
      return;
    }
    if (event.key === "Delete") {
      event.preventDefault();
      askDelete();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a" && !hits) {
      event.preventDefault();
      selection = visible.map((e) => e.id);
    } else if (event.key === "Escape" && selection.length > 0) {
      event.preventDefault();
      selectOnly(null);
    } else if (event.key === "Backspace" || (event.altKey && event.key === "ArrowUp")) {
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

<svelte:window {onkeydown} {onfocus} />

<div class="explorer">
  <header class="toolbar">
    <button
      type="button"
      class="tool"
      onclick={() => (panels.list = !panels.list)}
      aria-pressed={panels.list}
      aria-label={panels.list ? "收起列表" : "展开列表"}
      title={panels.list ? "收起列表" : "展开列表"}
      ><svg viewBox="0 0 20 16" width="17" height="14" aria-hidden="true"
        ><rect
          x="1"
          y="1.5"
          width="18"
          height="13"
          rx="3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
        ></rect><path d="M7 1.5v13" stroke="currentColor" stroke-width="1.5"
        ></path>{#if panels.list}<rect
            x="2.5"
            y="3"
            width="3.5"
            height="10"
            rx="1.2"
            fill="currentColor"
            opacity="0.45"
          ></rect>{/if}</svg
      ></button
    >
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
    <button
      type="button"
      class="tool"
      class:on={dupOpen}
      onclick={() => (dupOpen = !dupOpen)}
      aria-pressed={dupOpen}
      aria-label="查找重复文件"
      title="在当前文件夹中查找重复文件"
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><rect
          x="2"
          y="4.5"
          width="8"
          height="10"
          rx="1.6"
          fill="none"
          stroke="currentColor"
          stroke-width="1.4"
        ></rect><path
          d="M5.5 2.5V2.3A.8.8 0 0 1 6.3 1.5h6.4a.8.8 0 0 1 .8.8v8.4a.8.8 0 0 1-.8.8h-.2"
          fill="none"
          stroke="currentColor"
          stroke-width="1.4"
          stroke-linecap="round"
        ></path></svg
      ></button
    >
    <button
      type="button"
      class="tool trash"
      onclick={() => askDelete()}
      disabled={selection.length === 0}
      aria-label="删除选中的 {selection.length} 项（Delete）"
      title="删除选中的项目（Delete）。Ctrl 或 Shift 点按可多选，Ctrl+A 全选"
      >{#if selection.length > 1}<span class="count">{selection.length}</span>{/if}<svg
        viewBox="0 0 24 24"
        width="16"
        height="16"
        aria-hidden="true"
        ><path
          d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m-9 0 1 12a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2l1-12"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        ></path></svg
      ></button
    >
    <button
      type="button"
      class="tool"
      class:spinning={refreshing}
      onclick={(e) => (e.shiftKey ? onrescan() : refresh(true))}
      aria-label="刷新"
      title="刷新：只读取上次扫描之后的变化。按住 Shift 点按则完整重新扫描"
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
    <button
      type="button"
      class="tool side-toggle"
      onclick={() => (panels.side = !panels.side)}
      aria-pressed={panels.side}
      aria-label={panels.side ? "收起存储空间" : "展开存储空间"}
      title={panels.side ? "收起存储空间" : "展开存储空间"}
      ><svg viewBox="0 0 20 16" width="17" height="14" aria-hidden="true"
        ><rect
          x="1"
          y="1.5"
          width="18"
          height="13"
          rx="3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
        ></rect><path d="M13 1.5v13" stroke="currentColor" stroke-width="1.5"
        ></path>{#if panels.side}<rect
            x="14"
            y="3"
            width="3.5"
            height="10"
            rx="1.2"
            fill="currentColor"
            opacity="0.45"
          ></rect>{/if}</svg
      ></button
    >
  </header>

  {#each [summary.fallbackReason ? `未能使用 MFT 直读（${summary.fallbackReason}），本次是普通遍历。以管理员身份运行会更快、更准确。` : "", filterError, error].filter(Boolean) as message (message)}
    <p class="banner" role="status">{message}</p>
  {/each}

  <div class="body">
    {#if dupOpen}
      <Duplicates
        rootId={dirId}
        rootName={title}
        {removed}
        ondelete={(ids) => askDelete(ids)}
        onreveal={revealNode}
        onclose={() => (dupOpen = false)}
      />
    {:else}
      <div class="fold list-fold" class:collapsed={!panels.list} inert={!panels.list}>
        <section class="panel list" aria-label="项目列表">
          {#if hits}
            <SearchResults
              query={searchedFor}
              {hits}
              onreveal={reveal}
              onclose={() => (hits = null)}
            />
          {:else}
            {#key dirId}
              <div
                class="slide"
                in:fly={{
                  x: direction * 28,
                  duration: direction === 0 ? 0 : 240,
                  easing: cubicOut,
                }}
              >
                <ChildListView
                  entries={filtered?.entries ?? all.entries}
                  total={filtered?.total ?? all.total}
                  parentSize={info?.size ?? 0}
                  {hoverId}
                  {selected}
                  focusId={selectedId}
                  onhover={(id) => (hoverId = id)}
                  onopen={open}
                  onselect={select}
                />
              </div>
            {/key}
          {/if}
        </section>
      </div>

      <section class="panel map" aria-label={panels.chart === "sunburst" ? "旭日图" : "矩形树图"}>
        <div class="map-head">
          {#if focus}
            <strong>{focus.name}</strong>
            <span
              >{formatSize(focus.size)} · {formatPercent(focus.size, info?.size ?? 0)}{focus.isDir
                ? ` · ${focus.files.toLocaleString()} 个文件`
                : ""}</span
            >
          {:else if selection.length > 1}
            <strong>已选 {selection.length} 项</strong>
            <span>{formatSize(selectedSize)} · {formatPercent(selectedSize, info?.size ?? 0)}</span>
          {:else}
            <span
              >{panels.chart === "sunburst"
                ? "点按一圈放大查看，点按圆心返回上一级；Ctrl 点按可多选"
                : "点按文件夹放大查看，Backspace 返回上一级；Ctrl 点按可多选"}</span
            >
          {/if}
          <div class="chart-switch" role="radiogroup" aria-label="图表类型">
            <button
              type="button"
              role="radio"
              aria-checked={panels.chart === "treemap"}
              title="矩形树图"
              onclick={() => (panels.chart = "treemap")}
              ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
                ><rect
                  x="1.5"
                  y="1.5"
                  width="13"
                  height="13"
                  rx="2.5"
                  fill="none"
                  stroke="currentColor"
                ></rect><path d="M8 1.5v13M8 8.5h6.5M11 8.5v6" stroke="currentColor"></path></svg
              ></button
            >
            <button
              type="button"
              role="radio"
              aria-checked={panels.chart === "sunburst"}
              title="旭日图"
              onclick={() => (panels.chart = "sunburst")}
              ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
                ><circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor"></circle><circle
                  cx="8"
                  cy="8"
                  r="2.6"
                  fill="none"
                  stroke="currentColor"
                ></circle><path
                  d="M8 1.5v3.9M13.6 11.2 10.2 9.3M2.4 11.2l3.4-1.9"
                  stroke="currentColor"
                ></path></svg
              ></button
            >
          </div>
        </div>
        {#if panels.chart === "sunburst"}
          <Sunburst
            {dirId}
            {version}
            {title}
            size={info?.size ?? 0}
            {hoverId}
            {selected}
            {highlight}
            onhover={(id) => (hoverId = id)}
            onopen={open}
            onselect={(id, toggle) => select(id, toggle ? "toggle" : "only")}
            onup={up}
          />
        {:else}
          <Treemap
            {dirId}
            {version}
            {entries}
            {hoverId}
            {selected}
            {highlight}
            onhover={(id) => (hoverId = id)}
            onopen={open}
            onselect={(id, toggle) => select(id, toggle ? "toggle" : "only")}
          />
        {/if}
      </section>

      <div class="fold side-fold" class:collapsed={!panels.side} inert={!panels.side}>
        <aside class="side">
          <TypeStats {stats} />
        </aside>
      </div>
    {/if}
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
      {(rootInfo?.files ?? summary.fileCount).toLocaleString()} 个文件，逻辑大小 {formatSize(
        rootInfo?.size ?? summary.totalSize,
      )}，占用 {formatSize(rootInfo?.allocated ?? summary.totalAllocated)}，扫描用时 {(
        summary.elapsedMs / 1000
      ).toFixed(1)} 秒{summary.skipped > 0
        ? `，${summary.skipped} 个文件夹无权读取`
        : ""}，{refreshNote ??
        (summary.fromCache
          ? `这是${formatAgo(summary.scannedAt)}的扫描结果`
          : `扫描于${formatAgo(summary.scannedAt)}`)}
    </div>
  </footer>
</div>

{#if plan}
  <ConfirmDelete
    {plan}
    busy={deleting}
    onconfirm={confirmDelete}
    oncancel={() => !deleting && (plan = null)}
  />
{/if}

{#if toast}
  <Toast
    message={toast.message}
    detail={toast.detail}
    tone={toast.tone}
    onclose={() => (toast = null)}
  />
{/if}

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
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
  .tool:hover:not(:disabled) {
    background: var(--hover);
    color: var(--ink);
  }

  .nav button:disabled,
  .tool:disabled {
    opacity: 0.3;
  }

  .spinning svg {
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .tool.on {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .tool[aria-pressed="true"] {
    color: var(--ink);
  }

  .trash {
    position: relative;
  }

  .count {
    position: absolute;
    top: -3px;
    right: -4px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    box-sizing: border-box;
    border-radius: 999px;
    background: var(--danger);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    line-height: 16px;
    font-variant-numeric: tabular-nums;
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
    --list-w: 400px;
    --side-w: 250px;
    flex: 1;
    min-height: 0;
    display: flex;
    padding: 12px 14px;
  }

  /* 可收起的侧栏：外层改宽度做动画，内容保持固定宽度，收起过程中不重新排版。 */
  .fold {
    flex: none;
    display: flex;
    min-height: 0;
    /* 收起时裁掉内容，但给面板阴影留出余地。 */
    overflow: clip;
    overflow-clip-margin: 24px;
    transition:
      width 0.32s cubic-bezier(0.32, 0.72, 0, 1),
      margin 0.32s cubic-bezier(0.32, 0.72, 0, 1),
      opacity 0.24s;
  }

  .list-fold {
    width: var(--list-w);
    margin-right: 12px;
    justify-content: flex-end;
  }

  .side-fold {
    width: var(--side-w);
    margin-left: 12px;
  }

  .fold.collapsed {
    width: 0;
    margin: 0;
    opacity: 0;
    overflow-clip-margin: 0;
  }

  .list-fold > .panel {
    flex: none;
    width: var(--list-w);
  }

  .map {
    flex: 1;
    min-width: 0;
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

  .chart-switch {
    display: flex;
    flex: none;
    gap: 2px;
    margin-left: auto;
    padding: 2px;
    border-radius: 7px;
    background: var(--field);
  }

  .chart-switch button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 20px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-2);
  }

  .chart-switch button[aria-checked="true"] {
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }

  .map-head strong {
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .map :global(.treemap),
  .map :global(.sunburst) {
    margin: 0 10px 10px;
  }

  .side {
    flex: none;
    width: var(--side-w);
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
      --list-w: 340px;
    }

    .side-fold,
    .side-toggle {
      display: none;
    }
  }
</style>

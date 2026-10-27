<script lang="ts">
  import { untrack } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import {
    cleanRules,
    cleanSuggestions,
    deleteItems,
    linkItems,
    listChildren,
    owners as fetchOwners,
    nodeInfo,
    planClean,
    planDelete,
    revealInExplorer,
    openInExplorer,
    type MoveResult,
    refreshScan,
    search,
    typeStats,
    bucketStats,
    type BucketStats,
    type ChildList,
    type DeletePlan,
    type LinkGroup,
    type NodeInfo,
    type ScanSummary,
    type SearchHit,
    type SelectMode,
    type TypeStat,
  } from "$lib/api";
  import { formatAgo, formatPercent, formatSize } from "$lib/format";
  import { setThemeMode, themeMode, type ThemeMode } from "$lib/theme";
  import { SvelteMap } from "svelte/reactivity";
  import ChildListView, {
    COLUMNS,
    DEFAULT_COLUMNS,
    type ColumnKey,
    type SortKey,
  } from "./ChildList.svelte";
  import ConfirmDelete from "./ConfirmDelete.svelte";
  import Duplicates from "./Duplicates.svelte";
  import Findings from "./Findings.svelte";
  import MoveDialog from "./MoveDialog.svelte";
  import Rankings from "./Rankings.svelte";
  import Similar from "./Similar.svelte";
  import Suggestions from "./Suggestions.svelte";
  import SearchResults from "./SearchResults.svelte";
  import Sunburst from "./Sunburst.svelte";
  import Toast from "./Toast.svelte";
  import Unreadable from "./Unreadable.svelte";
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
  type View = "browse" | "suggest" | "rank" | "dupes" | "similar" | "findings";

  /** 左侧边栏的各个页面；图标是 16×16 的描边路径。 */
  const VIEWS: { key: View; label: string; icon: string }[] = [
    {
      key: "browse",
      label: "概览",
      icon: "M2.5 2.5h5v6h-5zM9.5 2.5h4v3h-4zM9.5 7.5h4v6h-4zM2.5 10.5h5v3h-5z",
    },
    {
      key: "suggest",
      label: "清理建议",
      icon: "M8 1.5v3M8 11.5v3M1.5 8h3M11.5 8h3M3.4 3.4l2.1 2.1M10.5 10.5l2.1 2.1M12.6 3.4l-2.1 2.1M5.5 10.5l-2.1 2.1",
    },
    { key: "rank", label: "大文件与排行", icon: "M3 13.5v-5M8 13.5v-11M13 13.5v-8" },
    { key: "dupes", label: "重复文件", icon: "M5.5 5h7v9h-7zM3.5 11.5V2.5h6.5" },
    {
      key: "similar",
      label: "相似图片",
      icon: "M2 3h12v10H2zM2 11l3.5-3.5 3 3 2-2L14 12M10.5 6.2h.01",
    },
    {
      key: "findings",
      label: "空文件夹与失效链接",
      icon: "M1.5 4.5a1 1 0 0 1 1-1h3.2l1.5 1.5h6.3a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z",
    },
  ];

  /** 收集篮里的一项。 */
  interface BasketItem {
    id: number;
    name: string;
    path: string;
    size: number;
    isDir: boolean;
  }

  /** 拖到收集篮时携带的数据类型：项目 id 的 JSON 数组。 */
  const DRAG_TYPE = "application/x-disklens-ids";

  /** 读取上次的侧栏展开状态和图表类型；存储不可用时用默认值。 */
  interface Panels {
    /** 左侧边栏展开（显示文字），收起时只显示图标。 */
    nav: boolean;
    list: boolean;
    side: boolean;
    chart: ChartKind;
    columns: ColumnKey[];
    sort: { key: SortKey; desc: boolean };
    /** 树图显示几层、旭日图画几圈。 */
    depth: { treemap: number; sunburst: number };
  }

  function loadPanels(): Panels {
    const defaults: Panels = {
      nav: true,
      list: true,
      side: true,
      chart: "treemap",
      columns: DEFAULT_COLUMNS,
      sort: { key: "size", desc: true },
      depth: { treemap: 1, sunburst: 5 },
    };
    try {
      const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}");
      return {
        nav: saved.nav !== false,
        list: saved.list !== false,
        side: saved.side !== false,
        chart: saved.chart === "sunburst" ? "sunburst" : "treemap",
        columns: Array.isArray(saved.columns) ? saved.columns : defaults.columns,
        sort: saved.sort?.key ? saved.sort : defaults.sort,
        depth: {
          treemap: [1, 2, 3].includes(saved.depth?.treemap) ? saved.depth.treemap : 1,
          sunburst: [3, 5, 7].includes(saved.depth?.sunburst) ? saved.depth.sunburst : 5,
        },
      };
    } catch {
      return defaults;
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
  let buckets = $state<BucketStats | null>(null);
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
  /** 确认后按规则清理（这些规则 id），而不是按项目删除。 */
  let planRules = $state<string[] | null>(null);
  let deleting = $state(false);
  let toast = $state<{ message: string; detail: string[]; tone: "info" | "warn" } | null>(null);
  let refreshing = $state(false);
  let menuOpen = $state(false);
  /** 正在查看读不到的文件夹。 */
  let unreadableOpen = $state(false);
  let theme = $state<ThemeMode>(themeMode());
  const THEMES: { key: ThemeMode; label: string }[] = [
    { key: "system", label: "跟随系统" },
    { key: "light", label: "浅色" },
    { key: "dark", label: "深色" },
  ];
  /** 正在移动的文件夹。 */
  let moving = $state<{ id: number; name: string; path: string; size: number } | null>(null);
  /** 左侧边栏选中的页面；除了概览，其他页面占据整个主区域。 */
  let view = $state<View>("browse");
  /** 清理建议能释放的空间，显示在边栏上。 */
  let reclaimable = $state(0);
  /** 收集篮：在各处挑出要删除的项目，最后一起删除。 */
  let basket = $state<BasketItem[]>([]);
  let dragOver = $state(false);
  /** 本次打开后删除过的项目，用来从重复文件结果里去掉。 */
  let removed = $state(new Set<number>());
  /** 最近一次增量刷新的结果，显示在状态栏。 */
  let refreshNote = $state<string | null>(null);
  let lastRefresh = 0;
  /** 窗口重新获得焦点时，距上次刷新至少这么久才再刷新。 */
  const FOCUS_REFRESH_MS = 5000;

  const entries = $derived(new Map(all.entries.map((e) => [e.id, e])));
  const basketSize = $derived(basket.reduce((sum, b) => sum + b.size, 0));
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
    const { key, desc } = panels.sort;
    Promise.all([nodeInfo(id), listChildren(id, null, key, desc), typeStats(id), bucketStats(id)])
      .then(([i, l, s, b]) => {
        if (mine !== loadSeq) return;
        info = i;
        buckets = b;
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
      listChildren(id, text, panels.sort.key, panels.sort.desc)
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

  // 边栏上显示清理建议能释放多少空间；删除或刷新后重新计算。
  $effect(() => {
    void version;
    cleanSuggestions()
      .then((list) => {
        reclaimable = list.filter((s) => s.action !== "guide").reduce((sum, s) => sum + s.size, 0);
      })
      .catch(() => {});
  });

  // 刷新后收集篮里的项目可能已被别的程序删除或改变大小：去掉不在的，更新大小。
  let basketVersion = untrack(() => version);
  $effect(() => {
    const v = version;
    if (v === basketVersion) return;
    basketVersion = v;
    const items = untrack(() => basket);
    if (items.length === 0) return;
    Promise.all(
      items.map((b) =>
        nodeInfo(b.id).then(
          (n) => ({ ...b, size: n.size, path: n.path }),
          () => null,
        ),
      ),
    ).then((list) => {
      const alive = new Map(list.flatMap((b) => (b ? [[b.id, b] as const] : [])));
      basket = basket.flatMap((b) =>
        items.includes(b) ? (alive.has(b.id) ? [alive.get(b.id)!] : []) : [b],
      );
    });
  });

  $effect(() => {
    const value = JSON.stringify(panels);
    try {
      localStorage.setItem(LAYOUT_KEY, value);
    } catch {
      // 存不下也不影响使用。
    }
  });

  /** 已查到的所有者；只查界面上看得见的行。 */
  const ownerCache = new SvelteMap<number, string | null>();

  function loadOwners(ids: number[]) {
    const missing = ids.filter((id) => !ownerCache.has(id));
    if (missing.length === 0) return;
    fetchOwners(missing)
      .then((names) => missing.forEach((id, i) => ownerCache.set(id, names[i])))
      .catch(() => missing.forEach((id) => ownerCache.set(id, null)));
  }

  /** 点列头排序：同一列再点一次反向；名称默认升序，其他默认降序。 */
  function sortBy(key: SortKey) {
    panels.sort =
      panels.sort.key === key ? { key, desc: !panels.sort.desc } : { key, desc: key !== "name" };
  }

  /**
   * 列表面板的宽度：名称列留 160px，加上每个显示的列和列间距，再加上图标列、
   * 选列按钮和左右边距（共 90px）。
   */
  const listWidth = $derived(
    250 +
      COLUMNS.filter((c) => panels.columns.includes(c.key))
        .map((c) => parseInt(c.width) + 8)
        .reduce((a, b) => a + b, 0),
  );

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

  /** 从统计面板点选：设好筛选条件；`deep` 时直接在所有子文件夹中查找。 */
  function applyFilter(query: string, deep: boolean) {
    filter = query;
    hits = null;
    if (deep && query) runSearch(query);
  }

  async function runSearch(text: string) {
    try {
      hits = await search(dirId, text, SEARCH_LIMIT);
      searchedFor = text;
      filterError = "";
    } catch (e) {
      filterError = String(e);
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

  /** 在图上定位一个搜索结果：进入它所在的文件夹并选中它，搜索结果保持打开。 */
  function reveal(hit: SearchHit) {
    if (hit.parent == null) return;
    const keep = hits;
    go(hit.parent, 0);
    selectOnly(hit.id);
    hits = keep;
  }

  /** `path` 是否就是 `dir` 或在它里面（Windows 路径不区分大小写）。 */
  function within(path: string, dir: string) {
    const p = path.toLowerCase();
    const d = dir.toLowerCase();
    return p === d || p.startsWith(d.endsWith("\\") ? d : d + "\\");
  }

  /** 加入收集篮。已在篮中文件夹里的项目不再重复加入；加入文件夹时去掉篮中它里面的项目。 */
  async function addToBasket(ids: number[]) {
    ctx = null;
    const fresh = ids.filter((id) => !basket.some((b) => b.id === id));
    if (fresh.length === 0) return;
    try {
      const infos = await Promise.all(fresh.map((id) => nodeInfo(id)));
      let next = [...basket];
      for (const n of infos) {
        if (n.parent == null || next.some((b) => within(n.path, b.path))) continue;
        next = next.filter((b) => !within(b.path, n.path));
        next.push({ id: n.id, name: n.name, path: n.path, size: n.size, isDir: n.isDir });
      }
      basket = next;
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  function removeFromBasket(id: number) {
    basket = basket.filter((b) => b.id !== id);
  }

  /** 从列表或图上拖动：拖的是选中的项目之一时带上全部选中项。 */
  function dragItems(id: number, event: DragEvent) {
    if (!event.dataTransfer) return;
    const ids = selected.has(id) ? selection : [id];
    event.dataTransfer.setData(DRAG_TYPE, JSON.stringify(ids));
    event.dataTransfer.effectAllowed = "copy";
  }

  function dropOnBasket(event: DragEvent) {
    dragOver = false;
    const data = event.dataTransfer?.getData(DRAG_TYPE);
    if (!data) return;
    event.preventDefault();
    try {
      const ids = JSON.parse(data);
      if (Array.isArray(ids)) addToBasket(ids.filter((x) => typeof x === "number"));
    } catch {
      // 不是我们拖出来的数据。
    }
  }

  /** 选中项先给出删除汇总，确认后再删。 */
  /** 按规则清理：先给出汇总，确认后执行。 */
  async function askClean(ruleIds: string[]) {
    if (ruleIds.length === 0 || plan) return;
    try {
      const p = await planClean(ruleIds);
      if (p.items.length === 0) {
        toast = { message: "所选项目里已经没有可以清理的内容", detail: [], tone: "info" };
        return;
      }
      planRules = ruleIds;
      plan = p;
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  async function askDelete(ids: number[] = selection) {
    if (ids.length === 0 || plan) return;
    try {
      planRules = null;
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
      const result = planRules
        ? await cleanRules(planRules, permanent)
        : await deleteItems(ids, permanent);
      const done = result.deleted.length;
      const verb = permanent ? "已彻底删除" : "已移到回收站";
      // 清理临时文件时常有大量正在使用的文件删不掉，只列出前几个。
      const failed = result.failed.slice(0, 5).map((f) => `${f.path}：${f.reason}`);
      if (result.failed.length > 5) {
        failed.push(`另有 ${result.failed.length - 5} 项没能删除，可能正被程序使用`);
      }
      toast = {
        message: done > 0 ? `${verb}，释放 ${formatSize(result.freed)}` : "没有删除任何项目",
        detail: [...failed, ...(result.logError ? [result.logError] : [])],
        tone: result.failed.length > 0 || result.logError ? "warn" : "info",
      };
      if (done > 0) {
        const gone = new Set(result.deleted);
        removed = new Set([...removed, ...gone]);
        selection = selection.filter((id) => !gone.has(id));
        basket = basket.filter((b) => !gone.has(b.id));
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
      planRules = null;
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

  /** 用硬链接替换重复文件，提示结果并刷新视图。 */
  async function linkDuplicates(groups: LinkGroup[]) {
    try {
      const r = await linkItems(groups);
      toast = {
        message:
          r.linked.length > 0
            ? `已替换 ${r.linked.length} 个文件为硬链接，释放 ${formatSize(r.freed)}`
            : "没有替换任何文件",
        detail: [
          ...r.failed.map((f) => `${f.path}：${f.reason}`),
          ...(r.logError ? [r.logError] : []),
        ],
        tone: r.failed.length > 0 || r.logError ? "warn" : "info",
      };
      if (r.linked.length > 0) {
        removed = new Set([...removed, ...r.linked]);
        rootInfo = await nodeInfo(summary.rootId);
        version++;
      }
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  /** “更多”菜单作用的项目：选中的一项，没有选中时是当前文件夹。 */
  const menuTarget = $derived(selection.length === 1 ? selection[0] : dirId);
  const menuEntry = $derived(selection.length === 1 ? entries.get(selection[0]) : undefined);

  /** 右键菜单：对象和弹出位置。 */
  let ctx = $state<{ id: number; isDir: boolean; x: number; y: number } | null>(null);

  async function openContext(id: number, event: MouseEvent) {
    if (!selected.has(id)) selectOnly(id);
    const entry = entries.get(id);
    let isDir = entry?.isDir ?? false;
    if (!entry) {
      try {
        isDir = (await nodeInfo(id)).isDir;
      } catch {
        return;
      }
    }
    // 不要超出窗口。
    const x = Math.min(event.clientX, window.innerWidth - 200);
    const y = Math.min(event.clientY, window.innerHeight - 200);
    ctx = { id, isDir, x, y };
  }

  function contextDelete(id: number) {
    ctx = null;
    askDelete(selection.includes(id) && selection.length > 1 ? selection : [id]);
  }

  async function askMove(id = menuEntry?.isDir ? menuEntry.id : undefined) {
    menuOpen = false;
    ctx = null;
    if (id == null) return;
    try {
      const n = await nodeInfo(id);
      if (!n.isDir) return;
      moving = { id: n.id, name: n.name, path: n.path, size: n.size };
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  async function moved(r: MoveResult) {
    moving = null;
    toast = {
      message: `已移到 ${r.dest}，这个磁盘释放 ${formatSize(r.bytes)}`,
      detail: [
        ...(r.leftover ? [`旧文件夹没能完全删除，可以手动删除：${r.leftover}`] : []),
        ...(r.logError ? [r.logError] : []),
      ],
      tone: r.leftover || r.logError ? "warn" : "info",
    };
    rootInfo = await nodeInfo(summary.rootId);
    version++;
    selectOnly(r.newId);
  }

  /** 在文件管理器中打开：文件夹打开它本身，文件打开所在文件夹并选中它。 */
  async function explore(id: number) {
    ctx = null;
    menuOpen = false;
    try {
      await openInExplorer(id);
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  async function showInExplorer(id = menuTarget) {
    menuOpen = false;
    ctx = null;
    try {
      await revealInExplorer(id);
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  async function copyPath(id = menuTarget) {
    menuOpen = false;
    ctx = null;
    try {
      const { path } = await nodeInfo(id);
      await navigator.clipboard.writeText(path);
      toast = { message: "已复制路径", detail: [path], tone: "info" };
    } catch (e) {
      toast = { message: String(e), detail: [], tone: "warn" };
    }
  }

  /** 关闭重复文件，在主界面中定位到 `id`。 */
  async function revealNode(id: number) {
    try {
      const n = await nodeInfo(id);
      view = "browse";
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
    if (["INPUT", "SELECT"].includes(target.tagName) || plan || moving || unreadableOpen) return;
    if ((menuOpen || ctx) && event.key === "Escape") {
      menuOpen = false;
      ctx = null;
      return;
    }
    if (view !== "browse") {
      if (event.key === "Escape") view = "browse";
      return;
    }
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      explore(selectedId ?? dirId);
    } else if (event.key === "Delete") {
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
    <div class="menu-wrap">
      <button
        type="button"
        class="tool"
        class:on={menuOpen}
        onclick={() => (menuOpen = !menuOpen)}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label="更多"
        title="更多"
        ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
          ><circle cx="3.5" cy="8" r="1.4" fill="currentColor"></circle><circle
            cx="8"
            cy="8"
            r="1.4"
            fill="currentColor"
          ></circle><circle cx="12.5" cy="8" r="1.4" fill="currentColor"></circle></svg
        ></button
      >
      {#if menuOpen}
        <div class="menu-shield" role="presentation" onclick={() => (menuOpen = false)}></div>
        <div class="menu" role="menu">
          <button
            type="button"
            role="menuitem"
            disabled={!menuEntry?.isDir}
            title={menuEntry?.isDir ? "" : "先选中一个文件夹"}
            onclick={() => askMove()}>移动到其他磁盘…</button
          >
          <button type="button" role="menuitem" onclick={() => showInExplorer()}
            >在资源管理器中显示</button
          >
          <button type="button" role="menuitem" onclick={() => copyPath()}>复制路径</button>
          <hr />
          <span class="menu-title">外观</span>
          {#each THEMES as t (t.key)}
            <button
              type="button"
              role="menuitemradio"
              aria-checked={theme === t.key}
              onclick={() => {
                theme = t.key;
                setThemeMode(t.key);
                menuOpen = false;
              }}
              ><span class="check" aria-hidden="true">{theme === t.key ? "✓" : ""}</span
              >{t.label}</button
            >
          {/each}
        </div>
      {/if}
    </div>
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

  <div class="main">
    <nav class="sidebar" class:narrow={!panels.nav} aria-label="功能">
      {#each VIEWS as v (v.key)}
        <button
          type="button"
          class:on={view === v.key}
          aria-current={view === v.key ? "page" : undefined}
          title={v.label}
          onclick={() => (view = v.key)}
          ><svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"
            ><path
              d={v.icon}
              fill="none"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linecap="round"
              stroke-linejoin="round"
            ></path></svg
          ><span class="label">{v.label}</span>{#if v.key === "suggest" && reclaimable > 0}<span
              class="badge">{formatSize(reclaimable)}</span
            >{/if}</button
        >
      {/each}
      <button
        type="button"
        class="collapse"
        onclick={() => (panels.nav = !panels.nav)}
        aria-label={panels.nav ? "收起边栏" : "展开边栏"}
        title={panels.nav ? "收起边栏" : "展开边栏"}
        ><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
          ><path
            d={panels.nav ? "M10 3.5 5.5 8l4.5 4.5" : "m6 3.5 4.5 4.5L6 12.5"}
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
          ></path></svg
        ></button
      >
    </nav>

    <div class="body" style:--list-w="{listWidth}px">
      {#if view !== "browse"}
        <div class="tools">
          {#if view === "suggest"}
            <Suggestions
              {version}
              onclean={askClean}
              onreveal={revealNode}
              onclose={() => (view = "browse")}
            />
          {:else if view === "rank"}
            <Rankings
              rootId={dirId}
              rootName={title}
              {removed}
              ondelete={(ids) => askDelete(ids)}
              onbasket={addToBasket}
              onreveal={revealNode}
              onclose={() => (view = "browse")}
            />
          {:else if view === "dupes"}
            <Duplicates
              rootId={dirId}
              rootName={title}
              {removed}
              ondelete={(ids) => askDelete(ids)}
              onbasket={addToBasket}
              onlink={linkDuplicates}
              onreveal={revealNode}
              onclose={() => (view = "browse")}
            />
          {:else if view === "similar"}
            <Similar
              rootId={dirId}
              rootName={title}
              {removed}
              ondelete={(ids) => askDelete(ids)}
              onbasket={addToBasket}
              onreveal={revealNode}
              onclose={() => (view = "browse")}
            />
          {:else}
            <Findings
              rootId={dirId}
              rootName={title}
              {removed}
              ondelete={(ids) => askDelete(ids)}
              onbasket={addToBasket}
              onreveal={revealNode}
              onclose={() => (view = "browse")}
            />
          {/if}
        </div>
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
                    columns={panels.columns}
                    sort={panels.sort}
                    owners={ownerCache}
                    onhover={(id) => (hoverId = id)}
                    onopen={open}
                    onselect={select}
                    onsort={sortBy}
                    oncolumns={(c) => {
                      panels.columns = c;
                    }}
                    onvisible={loadOwners}
                    oncontext={(id, e) => openContext(id, e)}
                    onexplore={explore}
                    ondragitem={dragItems}
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
            {:else if hits}
              <strong>搜索“{searchedFor}”</strong>
              <span>有命中项的方块保持原色，其余变灰</span>
            {:else if selection.length > 1}
              <strong>已选 {selection.length} 项</strong>
              <span
                >{formatSize(selectedSize)} · {formatPercent(selectedSize, info?.size ?? 0)}</span
              >
            {:else}
              <span
                >{panels.chart === "sunburst"
                  ? "点按一圈放大查看，点按圆心返回上一级；Ctrl 点按可多选"
                  : "点按文件夹放大查看，Backspace 返回上一级；Ctrl 点按可多选"}</span
              >
            {/if}
            <select
              class="depth"
              aria-label={panels.chart === "sunburst" ? "旭日图圈数" : "树图层级"}
              title={panels.chart === "sunburst" ? "旭日图圈数" : "树图显示几层"}
              value={panels.chart === "sunburst" ? panels.depth.sunburst : panels.depth.treemap}
              onchange={(e) => {
                const v = Number((e.currentTarget as HTMLSelectElement).value);
                if (!v) return;
                if (panels.chart === "sunburst") panels.depth.sunburst = v;
                else panels.depth.treemap = v;
              }}
            >
              {#each panels.chart === "sunburst" ? [3, 5, 7] : [1, 2, 3] as n (n)}
                <option value={n}>{panels.chart === "sunburst" ? `${n} 圈` : `${n} 层`}</option>
              {/each}
            </select>
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
              query={hits ? searchedFor : null}
              onhover={(id) => (hoverId = id)}
              onopen={open}
              onselect={(id, toggle) => select(id, toggle ? "toggle" : "only")}
              onup={up}
              depth={panels.depth.sunburst}
              oncontext={openContext}
              onexplore={explore}
              ondragitem={dragItems}
            />
          {:else}
            <Treemap
              {dirId}
              {version}
              {entries}
              {hoverId}
              {selected}
              {highlight}
              query={hits ? searchedFor : null}
              onhover={(id) => (hoverId = id)}
              onopen={open}
              onselect={(id, toggle) => select(id, toggle ? "toggle" : "only")}
              depth={panels.depth.treemap}
              oncontext={openContext}
              onexplore={explore}
              ondragitem={dragItems}
            />
          {/if}
        </section>

        <div class="fold side-fold" class:collapsed={!panels.side} inert={!panels.side}>
          <aside class="side">
            <TypeStats {stats} {buckets} {filter} onfilter={applyFilter} />
          </aside>
        </div>
      {/if}
    </div>
  </div>

  <div
    class="basket"
    class:over={dragOver}
    role="region"
    aria-label="收集篮"
    ondragover={(event) => {
      if (!event.dataTransfer?.types.includes(DRAG_TYPE)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      dragOver = true;
    }}
    ondragleave={() => (dragOver = false)}
    ondrop={dropOnBasket}
  >
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
      ><path
        d="M2 6h12l-1.3 7.2a1 1 0 0 1-1 .8H4.3a1 1 0 0 1-1-.8zM5 6l2.2-3.5M11 6 8.8 2.5"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
        stroke-linejoin="round"
      ></path></svg
    >
    <strong>收集篮</strong>
    {#if basket.length === 0}
      <span class="hint">把要删除的项目拖到这里，或右键选择“加入收集篮”，收集好了一起删除</span>
    {:else}
      <span class="sum">{basket.length} 项 · {formatSize(basketSize)}</span>
      <div class="chips">
        {#each basket as b (b.id)}
          <span class="chip" title={b.path}>
            <button type="button" class="chip-name" onclick={() => revealNode(b.id)}
              >{b.name}</button
            ><button
              type="button"
              class="chip-x"
              aria-label="把 {b.name} 移出收集篮"
              onclick={() => removeFromBasket(b.id)}
              ><svg viewBox="0 0 16 16" width="9" height="9" aria-hidden="true"
                ><path
                  d="m4 4 8 8M12 4l-8 8"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                ></path></svg
              ></button
            >
          </span>
        {/each}
      </div>
      <button type="button" class="basket-btn" onclick={() => (basket = [])}>清空</button>
      <button
        type="button"
        class="basket-btn danger"
        onclick={() => askDelete(basket.map((b) => b.id))}>删除全部…</button
      >
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
      ).toFixed(1)} 秒{#if summary.skipped > 0}，<button
          type="button"
          class="status-link"
          title="查看读不到的文件夹和原因"
          onclick={() => (unreadableOpen = true)}
          >{summary.skipped.toLocaleString()} 个文件夹读不到</button
        >{/if}，{refreshNote ??
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

{#if ctx}
  {@const c = ctx}
  <div
    class="menu-shield"
    role="presentation"
    onclick={() => (ctx = null)}
    oncontextmenu={(e) => {
      e.preventDefault();
      ctx = null;
    }}
  ></div>
  <div class="menu context" role="menu" style:left="{c.x}px" style:top="{c.y}px">
    <button type="button" role="menuitem" onclick={() => explore(c.id)}
      >{c.isDir ? "在文件管理器中打开" : "打开所在位置"}<span class="key">中键</span></button
    >
    {#if c.isDir}
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          ctx = null;
          open(c.id);
        }}>打开</button
      >
    {/if}
    {#if c.isDir}
      <button type="button" role="menuitem" onclick={() => showInExplorer(c.id)}
        >在上级文件夹中显示</button
      >
    {/if}
    <button type="button" role="menuitem" onclick={() => copyPath(c.id)}>复制路径</button>
    {#if c.isDir}
      <button type="button" role="menuitem" onclick={() => askMove(c.id)}>移动到其他磁盘…</button>
    {/if}
    <hr />
    <button
      type="button"
      role="menuitem"
      onclick={() =>
        addToBasket(selection.includes(c.id) && selection.length > 1 ? selection : [c.id])}
      >{selection.includes(c.id) && selection.length > 1
        ? `把选中的 ${selection.length} 项加入收集篮`
        : "加入收集篮"}</button
    >
    <button type="button" role="menuitem" class="danger" onclick={() => contextDelete(c.id)}
      >{selection.includes(c.id) && selection.length > 1
        ? `删除选中的 ${selection.length} 项…`
        : "删除…"}</button
    >
  </div>
{/if}

{#if unreadableOpen}
  <Unreadable
    root={summary.root}
    total={summary.skipped}
    dirs={summary.unreadable}
    onreveal={(id) => {
      unreadableOpen = false;
      revealNode(id);
    }}
    onexplore={explore}
    onclose={() => (unreadableOpen = false)}
  />
{/if}

{#if moving}
  <MoveDialog
    id={moving.id}
    name={moving.name}
    path={moving.path}
    size={moving.size}
    onmoved={moved}
    onclose={() => (moving = null)}
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

  .menu-wrap {
    position: relative;
  }

  .menu-shield {
    position: fixed;
    inset: 0;
    z-index: 14;
  }

  .menu {
    position: absolute;
    top: 34px;
    right: 0;
    z-index: 15;
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: 5px;
    border-radius: 10px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
  }

  .menu button {
    padding: 6px 10px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
  }

  .menu button:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-ink);
  }

  .menu button:disabled {
    opacity: 0.4;
  }

  .menu.context {
    position: fixed;
    top: auto;
    right: auto;
  }

  .menu-title {
    padding: 4px 10px 2px;
    color: var(--ink-3);
    font-size: 11px;
  }

  .menu .check {
    display: inline-block;
    width: 16px;
  }

  .menu .key {
    float: right;
    margin-left: 16px;
    opacity: 0.55;
    font-size: 11px;
  }

  .menu hr {
    width: 100%;
    margin: 4px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }

  .menu .danger {
    color: var(--danger);
  }

  .menu .danger:hover {
    background: var(--danger);
    color: #fff;
  }

  .depth {
    height: 22px;
    margin-left: auto;
    padding: 0 4px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink-2);
    font: inherit;
    font-size: 12px;
  }

  .tools {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    min-height: 0;
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

  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .sidebar {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 196px;
    padding: 12px 0 10px 12px;
    box-sizing: border-box;
    transition: width 0.28s cubic-bezier(0.32, 0.72, 0, 1);
  }

  .sidebar button {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--ink-2);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
  }

  .sidebar button svg {
    flex: none;
  }

  .sidebar button:hover {
    background: var(--hover);
    color: var(--ink);
  }

  .sidebar button.on {
    background: var(--panel);
    color: var(--ink);
    font-weight: 600;
    box-shadow: var(--shadow);
  }

  .sidebar button.on svg {
    color: var(--accent);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .badge {
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 10.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .sidebar .collapse {
    margin-top: auto;
    width: 32px;
    justify-content: center;
    padding: 0;
  }

  .sidebar.narrow {
    width: 56px;
  }

  .sidebar.narrow .label,
  .sidebar.narrow .badge {
    display: none;
  }

  .sidebar.narrow button {
    width: 36px;
    justify-content: center;
    padding: 0;
  }

  .basket {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    margin: 0 14px 10px;
    padding: 0 8px 0 12px;
    border: 1.5px dashed var(--line);
    border-radius: 10px;
    color: var(--ink-2);
    font-size: 12px;
    transition:
      background-color 0.15s,
      border-color 0.15s;
  }

  .basket.over {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .basket strong {
    color: var(--ink);
    white-space: nowrap;
  }

  .basket .hint {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .basket .sum {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .chips {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: thin;
  }

  .chip {
    flex: none;
    display: inline-flex;
    align-items: center;
    max-width: 220px;
    height: 24px;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
  }

  .chip-name {
    min-width: 0;
    padding: 0 4px 0 8px;
    border: 0;
    background: none;
    color: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-name:hover {
    text-decoration: underline;
  }

  .chip-x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-right: 2px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink-2);
  }

  .chip-x:hover {
    background: var(--hover);
    color: var(--ink);
  }

  .basket-btn {
    flex: none;
    height: 26px;
    padding: 0 12px;
    border: 0;
    border-radius: 7px;
    background: var(--field);
    color: var(--ink);
  }

  .basket-btn.danger {
    background: var(--danger);
    color: #fff;
    font-weight: 500;
  }

  .body {
    --list-w: 400px;
    --side-w: 250px;
    flex: 1;
    min-width: 0;
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
    flex: none;
    max-width: 60%;
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .map-head span {
    min-width: 0;
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

  .status-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--warn);
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
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

    .sidebar {
      width: 56px;
    }

    .sidebar .label,
    .sidebar .badge {
      display: none;
    }

    .sidebar button {
      width: 36px;
      justify-content: center;
      padding: 0;
    }

    .side-fold,
    .side-toggle {
      display: none;
    }
  }
</style>

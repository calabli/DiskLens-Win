<script lang="ts">
  import { untrack } from "svelte";
  import { treemap, type Entry, type TreemapRect } from "$lib/api";
  import { assignShades, resolvePalette, type Palette } from "$lib/categories";
  import { formatSize } from "$lib/format";

  interface Props {
    dirId: number;
    /** 数据版本；删除文件后递增，促使重新取布局。 */
    version: number;
    /** 显示几层：1 只画直接子项，更大时在目录块里继续画它的子项。 */
    depth: number;
    /** 当前目录子项，按 id 索引，用来取名称和大小。 */
    entries: Map<number, Entry>;
    hoverId: number | null;
    /** 选中的项目（可多选）。 */
    selected: Set<number>;
    /** 筛选时只高亮这些 id，其余变淡；`null` 表示不筛选。 */
    highlight: Set<number> | null;
    onhover: (id: number | null) => void;
    onopen: (id: number) => void;
    /** `toggle` 为 true 时（Ctrl+点按）加入或移出选择，否则只选这一项。 */
    onselect: (id: number, toggle: boolean) => void;
    oncontext: (id: number, event: MouseEvent) => void;
    /** 鼠标中键：在文件管理器中打开。 */
    onexplore: (id: number) => void;
  }

  let {
    dirId,
    version,
    depth,
    entries,
    hoverId,
    selected,
    highlight,
    onhover,
    onopen,
    onselect,
    oncontext,
    onexplore,
  }: Props = $props();

  interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
  }

  /**
   * 缩放过渡。`outer` 是上一层的布局，`focus` 是其中被进入 / 刚离开的那一块，
   * `inner` 是这一块里面的布局。z = 0 时看到整个 outer，z = 1 时 focus 铺满画布、看到 inner。
   * 进入：z 从 0 到 1；返回：z 从 1 到 0。`kind = "fade"` 时做淡入淡出。
   */
  interface Transition {
    kind: "zoom" | "fade";
    outer: TreemapRect[];
    inner: TreemapRect[] | null;
    focus: Box;
    from: number;
    to: number;
    start: number;
  }

  const DURATION = 460;
  const reduceMotion =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  let container: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  /** 当前显示的布局（命中测试用）。 */
  let rects = $state<TreemapRect[]>([]);
  let error = $state("");
  /** 重绘计数，深浅色切换等需要重画时递增。 */
  let repaint = $state(0);
  /** 已拿到布局的目录，用来判断“空文件夹”提示。 */
  let loadedDir = $state(-1);

  // 以下不是响应式状态：动画逐帧读写。
  let shownDir = -1;
  let layoutSize = { w: 0, h: 0 };
  let transition: Transition | null = null;
  let frame = 0;
  let palette: Palette | null = null;

  $effect(() => {
    const ro = new ResizeObserver(([e]) => {
      width = Math.floor(e.contentRect.width);
      height = Math.floor(e.contentRect.height);
    });
    ro.observe(container);
    const scheme = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => {
      palette = null;
      repaint++;
    };
    scheme.addEventListener("change", onScheme);
    return () => {
      ro.disconnect();
      scheme.removeEventListener("change", onScheme);
      cancelAnimationFrame(frame);
    };
  });

  // 目录或尺寸变化：向 Rust 要新布局，并决定用哪种过渡。
  let request = 0;
  $effect(() => {
    const id = dirId;
    const w = width;
    const h = height;
    void version;
    void depth;
    if (w < 10 || h < 10) return;
    const mine = ++request;
    const navigating = id !== shownDir;
    const previous = untrack(() => rects);
    const from = shownDir;
    const sameSize = layoutSize.w === w && layoutSize.h === h;
    const animate = navigating && sameSize && !reduceMotion && previous.length > 0;

    // 进入子目录：旧布局就在手上，立即开始放大，不必等新布局。
    const focus = animate ? previous.find((r) => r.id === id) : undefined;
    if (focus) {
      begin({ kind: "zoom", outer: previous, inner: null, focus, from: 0, to: 1 });
    }

    // 尺寸变化时稍作防抖；切换目录时立即请求。
    const timer = setTimeout(
      async () => {
        try {
          const next = await treemap(id, w, h, depth);
          if (mine !== request) return;
          if (focus && transition?.kind === "zoom" && transition.to === 1) {
            transition.inner = next;
          } else if (animate) {
            const back = next.find((r) => r.id === from);
            begin(
              back
                ? { kind: "zoom", outer: next, inner: previous, focus: back, from: 1, to: 0 }
                : { kind: "fade", outer: next, inner: previous, focus: full(), from: 0, to: 1 },
            );
          }
          rects = next;
          shownDir = id;
          loadedDir = id;
          layoutSize = { w, h };
          error = "";
          if (!transition) paint();
        } catch (e) {
          if (mine === request) error = String(e);
        }
      },
      navigating ? 0 : 60,
    );
    return () => clearTimeout(timer);
  });

  // 静态重绘：悬停、选中、筛选、名称或配色变化时。
  $effect(() => {
    void [rects, width, height, hoverId, selected, highlight, entries, repaint];
    if (!transition) untrack(paint);
  });

  function full(): Box {
    return { x: 0, y: 0, w: width, h: height };
  }

  function begin(t: Omit<Transition, "start">) {
    transition = { ...t, start: performance.now() };
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(tick);
  }

  /** 先快后慢，接近 macOS 的缩放手感。 */
  function ease(p: number) {
    return 1 - Math.pow(1 - p, 4);
  }

  function tick(now: number) {
    const t = transition;
    if (!t) return;
    const p = Math.min(1, (now - t.start) / DURATION);
    const z = t.from + (t.to - t.from) * ease(p);
    paint(z);
    // 放大结束时如果新布局还没到，停在放大状态等它。
    const waiting = t.kind === "zoom" && t.to === 1 && t.inner === null;
    if (p < 1 || waiting) {
      frame = requestAnimationFrame(tick);
    } else {
      transition = null;
      paint();
    }
  }

  function ensureCanvas(ctx: CanvasRenderingContext2D, w: number, h: number) {
    const dpr = window.devicePixelRatio || 1;
    const pw = Math.round(w * dpr);
    const ph = Math.round(h * dpr);
    if (canvas.width !== pw || canvas.height !== ph) {
      canvas.width = pw;
      canvas.height = ph;
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
  }

  function tilePath(ctx: CanvasRenderingContext2D, b: Box, inset: number) {
    const x = b.x + inset;
    const y = b.y + inset;
    const w = b.w - 2 * inset;
    const h = b.h - 2 * inset;
    ctx.beginPath();
    if (w <= 0 || h <= 0) return false;
    const radius = Math.min(7, w / 4, h / 4);
    if (radius < 1) ctx.rect(x, y, w, h);
    else ctx.roundRect(x, y, w, h, radius);
    return true;
  }

  // 每份布局只算一次深浅分配。
  const shadeCache = new WeakMap<TreemapRect[], Uint8Array>();
  function shadesOf(list: TreemapRect[]) {
    let s = shadeCache.get(list);
    if (!s) shadeCache.set(list, (s = assignShades(list)));
    return s;
  }

  function drawTiles(
    ctx: CanvasRenderingContext2D,
    list: TreemapRect[],
    map: (r: TreemapRect) => Box,
    alpha: number,
    pal: Palette,
    dim: Set<number> | null,
  ) {
    // 嵌套的块排在所属的顶层块后面，筛选时跟着顶层块一起变淡。
    let top = -1;
    const pick = shadesOf(list);
    list.forEach((r, i) => {
      if (r.level <= 1) top = r.id;
      const b = map(r);
      if (b.w < 0.5 || b.h < 0.5) return;
      ctx.globalAlpha = alpha * (dim && !dim.has(top) ? 0.22 : 1);
      ctx.fillStyle = pal.fill[r.category][pick[i]];
      if (tilePath(ctx, b, b.w > 6 && b.h > 6 ? 1.5 : 0.5)) ctx.fill();
    });
    ctx.globalAlpha = 1;
  }

  function ellipsize(ctx: CanvasRenderingContext2D, text: string, max: number): string {
    if (ctx.measureText(text).width <= max) return text;
    let lo = 0;
    let hi = text.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (ctx.measureText(text.slice(0, mid) + "…").width <= max) lo = mid;
      else hi = mid - 1;
    }
    return lo > 0 ? text.slice(0, lo) + "…" : "";
  }

  /** 画一帧。`z` 为 undefined 时画静态画面（带标签、悬停和选中）。 */
  function paint(z?: number) {
    if (!canvas || width === 0 || height === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    palette ??= resolvePalette(container);
    const pal = palette;
    ensureCanvas(ctx, width, height);
    const area = full();
    const t = transition;

    if (t && z !== undefined) {
      if (t.kind === "fade") {
        if (t.inner) drawTiles(ctx, t.inner, (r) => r, 1 - z, pal, null);
        drawTiles(ctx, t.outer, (r) => r, z, pal, null);
        return;
      }
      // 视口从整个画布插值到 focus，再把视口映射回整个画布。
      const view: Box = {
        x: t.focus.x * z,
        y: t.focus.y * z,
        w: area.w + (t.focus.w - area.w) * z,
        h: area.h + (t.focus.h - area.h) * z,
      };
      const sx = area.w / view.w;
      const sy = area.h / view.h;
      const toScreen = (r: Box): Box => ({
        x: (r.x - view.x) * sx,
        y: (r.y - view.y) * sy,
        w: r.w * sx,
        h: r.h * sy,
      });
      drawTiles(ctx, t.outer, toScreen, 1, pal, null);
      if (t.inner) {
        const slot = toScreen(t.focus);
        const into = (r: Box): Box => ({
          x: slot.x + (r.x / area.w) * slot.w,
          y: slot.y + (r.y / area.h) * slot.h,
          w: (r.w / area.w) * slot.w,
          h: (r.h / area.h) * slot.h,
        });
        const fade = Math.min(1, Math.max(0, (z - 0.35) / 0.65));
        ctx.globalAlpha = fade;
        ctx.fillStyle = pal.gap;
        if (tilePath(ctx, slot, 1.5)) ctx.fill();
        drawTiles(ctx, t.inner, into, fade, pal, null);
      }
      return;
    }

    // 侧栏展开、收起时画布尺寸逐帧变化，新布局要等尺寸稳定后才取；期间先把旧布局拉伸铺满。
    const sx = layoutSize.w > 0 ? width / layoutSize.w : 1;
    const sy = layoutSize.h > 0 ? height / layoutSize.h : 1;
    const fit = (r: Box): Box => ({ x: r.x * sx, y: r.y * sy, w: r.w * sx, h: r.h * sy });
    drawTiles(ctx, rects, fit, 1, pal, highlight);
    ctx.textBaseline = "top";
    const family = getComputedStyle(container).fontFamily;
    for (const rect of rects) {
      const r = fit(rect);
      const entry = rect.level === 1 ? entries.get(rect.id) : undefined;
      if (!entry || r.w < 52 || r.h < 24) continue;
      if (highlight && !highlight.has(rect.id)) continue;
      ctx.fillStyle = pal.ink;
      // 嵌套显示时，目录块顶部的名称栏里放名称和大小，下面是它的子项。
      if (nestedIn(rect)) {
        ctx.font = `600 11px ${family}`;
        const label = `${entry.name}  ${formatSize(entry.size)}`;
        ctx.fillText(ellipsize(ctx, label, r.w - 10), r.x + 5, r.y + 4);
        continue;
      }
      ctx.font = `600 12px ${family}`;
      ctx.fillText(ellipsize(ctx, entry.name, r.w - 16), r.x + 8, r.y + 7);
      if (r.h > 40) {
        ctx.globalAlpha = 0.72;
        ctx.font = `12px ${family}`;
        ctx.fillText(formatSize(entry.size), r.x + 8, r.y + 23);
        ctx.globalAlpha = 1;
      }
    }

    const hovered = rects.find((r) => r.id === hoverId);
    if (hovered && tilePath(ctx, fit(hovered), 1.5)) {
      ctx.fillStyle = "rgb(255 255 255 / 0.22)";
      ctx.fill();
    }
    ctx.strokeStyle = pal.accent;
    ctx.lineWidth = 2.5;
    for (const r of rects) {
      if (selected.has(r.id) && tilePath(ctx, fit(r), 2.5)) ctx.stroke();
    }
  }

  /** 这个目录块里是否画了子项（与 Rust 侧的 NEST_MIN 一致）。 */
  function nestedIn(r: TreemapRect) {
    return depth > 1 && r.isDir && r.w >= 48 && r.h >= 48;
  }

  /** 鼠标下最里层的块：子块排在父块后面，所以从后往前找。 */
  function hit(event: MouseEvent): TreemapRect | undefined {
    if (transition) return undefined;
    const box = canvas.getBoundingClientRect();
    const x = event.clientX - box.left;
    const y = event.clientY - box.top;
    return rects.findLast((r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
  }

  function onmove(event: MouseEvent) {
    const id = hit(event)?.id ?? null;
    if (id !== hoverId) onhover(id);
  }

  function onclick(event: MouseEvent) {
    const r = hit(event);
    if (!r) return;
    const toggle = event.ctrlKey || event.metaKey;
    // 画出来的目录大小都不为 0，一定有子项，可以进入。
    if (!toggle && r.isDir) onopen(r.id);
    else onselect(r.id, toggle);
  }

  function onauxclick(event: MouseEvent) {
    if (event.button !== 1) return;
    event.preventDefault();
    const r = hit(event);
    if (r) onexplore(r.id);
  }

  function oncontextmenu(event: MouseEvent) {
    event.preventDefault();
    const r = hit(event);
    if (r) oncontext(r.id, event);
  }
</script>

<div class="treemap" bind:this={container}>
  <!-- 树图只供鼠标浏览；左侧列表是它可用键盘和读屏软件操作的等价视图。 -->
  <canvas
    bind:this={canvas}
    aria-hidden="true"
    onmousemove={onmove}
    onmouseleave={() => onhover(null)}
    {onclick}
    {oncontextmenu}
    {onauxclick}
    onmousedown={(e) => e.button === 1 && e.preventDefault()}
  ></canvas>
  {#if error}
    <p class="note error" role="alert">{error}</p>
  {:else if rects.length === 0 && width > 0 && loadedDir === dirId}
    <p class="note">这个文件夹是空的</p>
  {/if}
</div>

<style>
  .treemap {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  canvas {
    position: absolute;
    inset: 0;
    cursor: pointer;
  }

  .note {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: var(--ink-3);
  }

  .error {
    color: var(--danger);
  }
</style>

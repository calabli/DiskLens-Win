<script lang="ts">
  import { untrack } from "svelte";
  import { sunburst, type SunburstSegment } from "$lib/api";
  import { resolvePalette, type Palette } from "$lib/categories";
  import { formatSize } from "$lib/format";

  interface Props {
    dirId: number;
    /** 数据版本；删除或刷新后递增，促使重新取布局。 */
    version: number;
    /** 画几圈。 */
    depth: number;
    /** 圆心显示的名称和大小。 */
    title: string;
    size: number;
    hoverId: number | null;
    /** 选中的项目（可多选）。 */
    selected: Set<number>;
    /** 筛选时只高亮这些直接子项（连同它们外圈的后代），其余变淡；`null` 表示不筛选。 */
    highlight: Set<number> | null;
    onhover: (id: number | null) => void;
    onopen: (id: number) => void;
    /** `toggle` 为 true 时（Ctrl+点按）加入或移出选择，否则只选这一项。 */
    onselect: (id: number, toggle: boolean) => void;
    /** 点按圆心：返回上一级。 */
    onup: () => void;
    oncontext: (id: number, event: MouseEvent) => void;
  }

  let {
    dirId,
    version,
    depth,
    title,
    size,
    hoverId,
    selected,
    highlight,
    onhover,
    onopen,
    onselect,
    onup,
    oncontext,
  }: Props = $props();
  /** 圆心空洞占半径的比例。 */
  const HOLE = 0.26;
  const DURATION = 460;
  const reduceMotion =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /**
   * 缩放过渡，思路同树图：`outer` 是上一层的布局，`focus` 是其中被进入 / 刚离开的那一段，
   * `inner` 是这一段里面的布局。z = 0 时看到整个 outer；z = 1 时 focus 张满一整圈并成为圆心，
   * 看到 inner。进入：z 从 0 到 1；返回：z 从 1 到 0。
   */
  interface Transition {
    kind: "zoom" | "fade";
    outer: SunburstSegment[];
    inner: SunburstSegment[] | null;
    focus: SunburstSegment;
    from: number;
    to: number;
    start: number;
  }

  /** 屏幕上的一段：角度为圆周比例，圈可以是小数（动画中）。 */
  interface Drawn {
    start: number;
    end: number;
    depth: number;
  }

  let container: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  let segments = $state<SunburstSegment[]>([]);
  let error = $state("");
  let repaint = $state(0);
  let loadedDir = $state(-1);

  let shownDir = -1;
  let transition: Transition | null = null;
  let frame = 0;
  let palette: Palette | null = null;

  const geometry = $derived.by(() => {
    const radius = Math.max(0, Math.min(width, height) / 2 - 6);
    const hole = radius * HOLE;
    const rings = Math.max(2, depth || 5);
    return { cx: width / 2, cy: height / 2, radius, hole, ring: (radius - hole) / rings };
  });

  /** 每段所属的第 1 圈祖先（按角度包含关系找），筛选时整支变淡。 */
  const topOf = $derived.by(() => {
    const tops = segments.filter((s) => s.depth === 1);
    const inside = (s: SunburstSegment, t: SunburstSegment) =>
      s.start >= t.start - 1e-6 && s.end <= t.end + 1e-6;
    return new Map(
      segments.map((s) => [s.id, (s.depth === 1 ? s : tops.find((t) => inside(s, t)))?.id]),
    );
  });

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

  // 目录变化：向 Rust 要新布局，并决定用哪种过渡。旭日图与尺寸无关，尺寸变化只需重画。
  let request = 0;
  $effect(() => {
    const id = dirId;
    void version;
    void depth;
    const mine = ++request;
    const navigating = id !== shownDir;
    const previous = untrack(() => segments);
    const from = shownDir;
    const animate = navigating && !reduceMotion && previous.length > 0 && shownDir !== -1;

    const focus = animate ? previous.find((s) => s.id === id) : undefined;
    if (focus) begin({ kind: "zoom", outer: previous, inner: null, focus, from: 0, to: 1 });

    sunburst(id, depth)
      .then((next) => {
        if (mine !== request) return;
        if (focus && transition?.kind === "zoom" && transition.to === 1) {
          transition.inner = next;
        } else if (animate) {
          const back = next.find((s) => s.id === from);
          begin(
            back
              ? { kind: "zoom", outer: next, inner: previous, focus: back, from: 1, to: 0 }
              : {
                  kind: "fade",
                  outer: next,
                  inner: previous,
                  focus: { id: -1, category: "other", isDir: true, depth: 0, start: 0, end: 1 },
                  from: 0,
                  to: 1,
                },
          );
        }
        segments = next;
        shownDir = id;
        loadedDir = id;
        error = "";
        if (!transition) paint();
      })
      .catch((e) => {
        if (mine === request) error = String(e);
      });
  });

  $effect(() => {
    void [segments, width, height, hoverId, selected, highlight, repaint];
    if (!transition) untrack(paint);
  });

  function begin(t: Omit<Transition, "start">) {
    transition = { ...t, start: performance.now() };
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(tick);
  }

  function ease(p: number) {
    return 1 - Math.pow(1 - p, 4);
  }

  function tick(now: number) {
    const t = transition;
    if (!t) return;
    const p = Math.min(1, (now - t.start) / DURATION);
    paint(t.from + (t.to - t.from) * ease(p));
    const waiting = t.kind === "zoom" && t.to === 1 && t.inner === null;
    if (p < 1 || waiting) {
      frame = requestAnimationFrame(tick);
    } else {
      transition = null;
      paint();
    }
  }

  function ensureCanvas(ctx: CanvasRenderingContext2D) {
    const dpr = window.devicePixelRatio || 1;
    const pw = Math.round(width * dpr);
    const ph = Math.round(height * dpr);
    if (canvas.width !== pw || canvas.height !== ph) {
      canvas.width = pw;
      canvas.height = ph;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
  }

  /** 画出一段弧的路径；太小画不出来时返回 false。 */
  function arcPath(ctx: CanvasRenderingContext2D, d: Drawn, inset = 0) {
    const { cx, cy, hole, ring, radius } = geometry;
    const inner = Math.max(hole, hole + (d.depth - 1) * ring) + inset;
    const outer = Math.min(radius, hole + d.depth * ring) - inset;
    const start = Math.max(0, d.start);
    const end = Math.min(1, d.end);
    ctx.beginPath();
    if (outer - inner < 0.5 || end - start <= 0) return false;
    if (end - start > 0.9999) {
      // 整圈画成圆环，免得在 12 点方向留下一条接缝。
      ctx.arc(cx, cy, outer, 0, Math.PI * 2);
      ctx.moveTo(cx + inner, cy);
      ctx.arc(cx, cy, inner, 0, Math.PI * 2, true);
      return true;
    }
    const a0 = start * Math.PI * 2 - Math.PI / 2;
    const a1 = end * Math.PI * 2 - Math.PI / 2;
    ctx.arc(cx, cy, outer, a0, a1);
    ctx.arc(cx, cy, inner, a1, a0, true);
    ctx.closePath();
    return true;
  }

  function drawSegments(
    ctx: CanvasRenderingContext2D,
    list: SunburstSegment[],
    map: (s: SunburstSegment) => Drawn,
    alpha: number,
    pal: Palette,
    dim: (s: SunburstSegment) => boolean,
  ) {
    ctx.strokeStyle = pal.gap;
    ctx.lineWidth = 1;
    list.forEach((s, i) => {
      const d = map(s);
      // 缩进圆心的段逐渐淡出。
      const fadeIn = Math.min(1, Math.max(0, d.depth));
      if (fadeIn <= 0) return;
      if (!arcPath(ctx, d)) return;
      ctx.globalAlpha = alpha * fadeIn * (dim(s) ? 0.22 : 1);
      const shades = pal.fill[s.category];
      ctx.fillStyle = shades[i % shades.length];
      ctx.fill();
      ctx.stroke();
    });
    ctx.globalAlpha = 1;
  }

  function paint(z?: number) {
    if (!canvas || width === 0 || height === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    palette ??= resolvePalette(container);
    const pal = palette;
    ensureCanvas(ctx);
    const t = transition;
    const same = (s: SunburstSegment): Drawn => s;
    const none = () => false;

    if (t && z !== undefined) {
      if (t.kind === "fade") {
        if (t.inner) drawSegments(ctx, t.inner, same, 1 - z, pal, none);
        drawSegments(ctx, t.outer, same, z, pal, none);
        return;
      }
      // 视窗从整圈插值到 focus 的角度范围，同时整体向内收 focus.depth 圈。
      const f = t.focus;
      const va = f.start * z;
      const vb = 1 + (f.end - 1) * z;
      const shift = f.depth * z;
      const view = (start: number, end: number, depth: number): Drawn => ({
        start: (start - va) / (vb - va),
        end: (end - va) / (vb - va),
        depth: depth - shift,
      });
      drawSegments(ctx, t.outer, (s) => view(s.start, s.end, s.depth), 1, pal, none);
      if (t.inner) {
        const span = f.end - f.start;
        const fade = Math.min(1, Math.max(0, (z - 0.35) / 0.65));
        drawSegments(
          ctx,
          t.inner,
          (s) => view(f.start + s.start * span, f.start + s.end * span, f.depth + s.depth),
          fade,
          pal,
          none,
        );
      }
      return;
    }

    const dim = (s: SunburstSegment) => highlight !== null && !highlight.has(topOf.get(s.id) ?? -1);
    drawSegments(ctx, segments, same, 1, pal, dim);

    const hovered = segments.find((s) => s.id === hoverId);
    if (hovered && arcPath(ctx, hovered)) {
      ctx.fillStyle = "rgb(255 255 255 / 0.22)";
      ctx.fill();
    }
    ctx.strokeStyle = pal.accent;
    ctx.lineWidth = 2.5;
    for (const s of segments) {
      if (selected.has(s.id) && arcPath(ctx, s, 1.5)) ctx.stroke();
    }
  }

  /** 鼠标位置对应的段；在圆心空洞里返回 "center"。 */
  function hit(event: MouseEvent): SunburstSegment | "center" | undefined {
    if (transition) return undefined;
    const box = canvas.getBoundingClientRect();
    const { cx, cy, hole, ring, radius } = geometry;
    const dx = event.clientX - box.left - cx;
    const dy = event.clientY - box.top - cy;
    const r = Math.hypot(dx, dy);
    if (r < hole) return "center";
    if (r > radius) return undefined;
    const depth = Math.floor((r - hole) / ring) + 1;
    let angle = (Math.atan2(dy, dx) + Math.PI / 2) / (Math.PI * 2);
    if (angle < 0) angle += 1;
    return segments.find((s) => s.depth === depth && angle >= s.start && angle < s.end);
  }

  let overCenter = $state(false);

  function onmove(event: MouseEvent) {
    const h = hit(event);
    overCenter = h === "center";
    const id = h && h !== "center" ? h.id : null;
    if (id !== hoverId) onhover(id);
  }

  function onclick(event: MouseEvent) {
    const h = hit(event);
    if (!h) return;
    if (h === "center") {
      onup();
      return;
    }
    const toggle = event.ctrlKey || event.metaKey;
    // 画出来的目录大小都不为 0，一定有子项，可以进入。
    if (!toggle && h.isDir) onopen(h.id);
    else onselect(h.id, toggle);
  }
</script>

<div class="sunburst" bind:this={container}>
  <!-- 与树图一样只供鼠标浏览；左侧列表是可用键盘和读屏软件操作的等价视图。 -->
  <canvas
    bind:this={canvas}
    aria-hidden="true"
    class:pointer={overCenter || hoverId !== null}
    onmousemove={onmove}
    onmouseleave={() => {
      overCenter = false;
      onhover(null);
    }}
    {onclick}
    oncontextmenu={(event) => {
      event.preventDefault();
      const h = hit(event);
      if (h && h !== "center") oncontext(h.id, event);
    }}
  ></canvas>
  {#if width > 0}
    <div
      class="center"
      class:hot={overCenter}
      style:width="{geometry.hole * 2 - 16}px"
      aria-hidden="true"
    >
      <strong>{title}</strong>
      <span>{formatSize(size)}</span>
    </div>
  {/if}
  {#if error}
    <p class="note error" role="alert">{error}</p>
  {:else if segments.length === 0 && width > 0 && loadedDir === dirId}
    <p class="note">这个文件夹是空的</p>
  {/if}
</div>

<style>
  .sunburst {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  canvas {
    position: absolute;
    inset: 0;
    cursor: default;
  }

  canvas.pointer {
    cursor: pointer;
  }

  .center {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    text-align: center;
    pointer-events: none;
    transition: opacity 0.15s;
  }

  .center strong {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .center span {
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .center.hot strong {
    color: var(--accent);
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

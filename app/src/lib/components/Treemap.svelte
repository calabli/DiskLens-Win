<script lang="ts">
  import { treemap, type Entry, type TreemapRect } from "$lib/api";
  import { CATEGORIES } from "$lib/categories";
  import { formatSize } from "$lib/format";

  interface Props {
    dirId: number;
    /** 当前目录子项，按 id 索引，用来取名称和大小。 */
    entries: Map<number, Entry>;
    hoverId: number | null;
    selectedId: number | null;
    /** 筛选时只高亮这些 id，其余变淡；`null` 表示不筛选。 */
    highlight: Set<number> | null;
    onhover: (id: number | null) => void;
    onopen: (id: number) => void;
    onselect: (id: number) => void;
  }

  let { dirId, entries, hoverId, selectedId, highlight, onhover, onopen, onselect }: Props =
    $props();

  let container: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  let rects = $state<TreemapRect[]>([]);
  let error = $state("");

  $effect(() => {
    const ro = new ResizeObserver(([e]) => {
      width = Math.floor(e.contentRect.width);
      height = Math.floor(e.contentRect.height);
    });
    ro.observe(container);
    return () => ro.disconnect();
  });

  // 目录或尺寸变化后向 Rust 要布局；拖动窗口时稍作防抖，只保留最后一次的结果。
  let request = 0;
  $effect(() => {
    const id = dirId;
    const w = width;
    const h = height;
    if (w < 10 || h < 10) return;
    const mine = ++request;
    const timer = setTimeout(async () => {
      try {
        const r = await treemap(id, w, h);
        if (mine === request) {
          rects = r;
          error = "";
        }
      } catch (e) {
        if (mine === request) error = String(e);
      }
    }, 50);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    draw(rects, width, height, hoverId, selectedId, highlight);
  });

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

  function draw(
    list: TreemapRect[],
    w: number,
    h: number,
    hover: number | null,
    selected: number | null,
    only: Set<number> | null,
  ) {
    if (!canvas || w === 0 || h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.textBaseline = "top";

    for (const r of list) {
      const style = CATEGORIES[r.category];
      const gap = r.w > 4 && r.h > 4 ? 1 : 0;
      ctx.globalAlpha = only && !only.has(r.id) ? 0.2 : 1;
      ctx.fillStyle = style.color;
      ctx.fillRect(r.x + gap, r.y + gap, r.w - 2 * gap, r.h - 2 * gap);

      const entry = entries.get(r.id);
      if (entry && r.w > 48 && r.h > 20) {
        ctx.fillStyle = style.ink;
        ctx.font = "600 12px 'Segoe UI', 'Microsoft YaHei UI', sans-serif";
        const name = ellipsize(ctx, entry.name, r.w - 12);
        ctx.fillText(name, r.x + 6, r.y + 5);
        if (r.h > 36) {
          ctx.font = "12px 'Segoe UI', 'Microsoft YaHei UI', sans-serif";
          ctx.fillText(formatSize(entry.size), r.x + 6, r.y + 21);
        }
      }
    }
    ctx.globalAlpha = 1;

    for (const [id, color, lineWidth] of [
      [selected, "#141413", 3],
      [hover, "#ffffff", 2],
    ] as const) {
      const r = id === null ? undefined : list.find((x) => x.id === id);
      if (!r) continue;
      ctx.strokeStyle = color;
      ctx.lineWidth = lineWidth;
      const inset = lineWidth / 2 + 1;
      ctx.strokeRect(r.x + inset, r.y + inset, r.w - 2 * inset, r.h - 2 * inset);
    }
  }

  function hit(event: MouseEvent): TreemapRect | undefined {
    const box = canvas.getBoundingClientRect();
    const x = event.clientX - box.left;
    const y = event.clientY - box.top;
    return rects.find((r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
  }

  function onmove(event: MouseEvent) {
    const id = hit(event)?.id ?? null;
    if (id !== hoverId) onhover(id);
  }

  function onclick(event: MouseEvent) {
    const r = hit(event);
    if (!r) return;
    const entry = entries.get(r.id);
    if (entry?.isDir && entry.hasChildren) onopen(r.id);
    else onselect(r.id);
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
  ></canvas>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if rects.length === 0 && width > 0}
    <p class="empty">这个目录是空的</p>
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

  .error,
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: var(--ink-2);
  }

  .error {
    color: var(--danger);
  }
</style>

<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import {
    cancelSimilar,
    findSimilar,
    thumbnailUrl,
    type SimilarImage,
    type SimilarResult,
  } from "$lib/api";
  import { formatSize } from "$lib/format";

  interface Props {
    rootId: number;
    rootName: string;
    /** 已经删除的项目，从结果里去掉。 */
    removed: Set<number>;
    ondelete: (ids: number[]) => void;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { rootId, rootName, removed, ondelete, onreveal, onclose }: Props = $props();

  const LEVELS = [
    { label: "几乎一样", value: 4 },
    { label: "相似", value: 8 },
    { label: "有点像", value: 12 },
  ];
  const SIZES = [
    { label: "≥ 20 KB", value: 20 << 10 },
    { label: "≥ 100 KB", value: 100 << 10 },
    { label: "≥ 1 MB", value: 1 << 20 },
    { label: "全部", value: 1 },
  ];
  /** 一次显示的组数，滚动到底再多显示。 */
  const PAGE = 40;
  /** 缩略图请求的长边像素（高分屏下显示为一半大小）。 */
  const THUMB = 320;

  let maxDistance = $state(8);
  let minSize = $state(20 << 10);
  let running = $state(false);
  let done = $state(0);
  let total = $state(0);
  let result = $state<SimilarResult | null>(null);
  let error = $state("");
  let shown = $state(PAGE);
  const marked = new SvelteSet<number>();

  const groups = $derived(
    (result?.groups ?? [])
      .map((g) => g.files.filter((f) => !removed.has(f.id)))
      .filter((files) => files.length > 1),
  );
  const markedFiles = $derived(groups.flat().filter((f) => marked.has(f.id)));
  const markedSize = $derived(markedFiles.reduce((sum, f) => sum + f.size, 0));

  /** 每组保留第一张（像素最多的），其余勾选。 */
  function markAllButBest() {
    marked.clear();
    for (const files of groups) for (const f of files.slice(1)) marked.add(f.id);
  }

  async function start() {
    running = true;
    error = "";
    result = null;
    done = 0;
    total = 0;
    shown = PAGE;
    marked.clear();
    try {
      result = await findSimilar(rootId, minSize, maxDistance, (d, t) => {
        done = d;
        total = t;
      });
      markAllButBest();
    } catch (e) {
      if (String(e) !== "已取消") error = String(e);
    } finally {
      running = false;
    }
  }

  function toggle(files: SimilarImage[], f: SimilarImage) {
    if (marked.has(f.id)) {
      marked.delete(f.id);
      return;
    }
    // 每组至少保留一张。
    if (files.every((x) => x.id === f.id || marked.has(x.id))) return;
    marked.add(f.id);
  }

  /** 进入可见区域时才向后台要缩略图，离开组件时释放。 */
  function thumb(img: HTMLImageElement, id: number) {
    let url = "";
    let gone = false;
    const io = new IntersectionObserver(
      ([e]) => {
        if (!e.isIntersecting) return;
        io.disconnect();
        thumbnailUrl(id, THUMB)
          .then((u) => {
            if (gone) URL.revokeObjectURL(u);
            else img.src = url = u;
          })
          .catch(() => img.classList.add("broken"));
      },
      { rootMargin: "200px" },
    );
    io.observe(img);
    return {
      destroy() {
        gone = true;
        io.disconnect();
        if (url) URL.revokeObjectURL(url);
      },
    };
  }

  function name(path: string) {
    return path.slice(path.lastIndexOf("\\") + 1);
  }

  function date(ms: number) {
    return new Date(ms).toLocaleDateString("zh-CN");
  }

  function onscroll(event: Event) {
    const el = event.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400 && shown < groups.length) {
      shown += PAGE;
    }
  }
</script>

<section class="similar" aria-label="相似图片">
  <header>
    <div class="title">
      <h2>相似图片</h2>
      <span>在 {rootName} 中查找看起来相同的图片：缩放、另存、重新压缩或调过色的都能找出</span>
    </div>
    <div class="levels" role="radiogroup" aria-label="相似程度">
      {#each LEVELS as l (l.value)}
        <button
          type="button"
          role="radio"
          aria-checked={maxDistance === l.value}
          disabled={running}
          onclick={() => (maxDistance = l.value)}>{l.label}</button
        >
      {/each}
    </div>
    <select bind:value={minSize} disabled={running} aria-label="最小文件大小">
      {#each SIZES as s (s.value)}
        <option value={s.value}>{s.label}</option>
      {/each}
    </select>
    {#if running}
      <button type="button" class="secondary" onclick={() => cancelSimilar()}>停止</button>
    {:else}
      <button type="button" class="primary" onclick={start}
        >{result ? "重新查找" : "开始查找"}</button
      >
    {/if}
    <button type="button" class="close" onclick={onclose} aria-label="关闭">
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"
        ><path
          d="m4 4 8 8M12 4l-8 8"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        ></path></svg
      >
    </button>
  </header>

  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}

  {#if running}
    <div class="state" aria-live="polite">
      <p>
        正在分析图片{total > 0 ? ` · ${done.toLocaleString()} / ${total.toLocaleString()}` : "…"}
      </p>
      <div class="progress" role="progressbar">
        <span style:width="{total > 0 ? (done / total) * 100 : 0}%"></span>
      </div>
      <p class="muted">第一次需要逐张解码，分析过的图片下次会快很多。</p>
    </div>
  {:else if !result}
    <div class="state">
      <p>支持 JPG、PNG、BMP、GIF、WebP。每组默认保留像素最多的那张。</p>
      <p class="muted">暂不支持 HEIC（iPhone 照片格式）。</p>
    </div>
  {:else if groups.length === 0}
    <div class="state">
      <p>{result.groups.length > 0 ? "相似图片都已处理。" : "没有找到相似的图片。"}</p>
      <p class="muted">
        分析了 {result.images.toLocaleString()} 张图片，用时 {(result.elapsedMs / 1000).toFixed(1)} 秒{result.skipped >
        0
          ? `，${result.skipped} 张无法读取`
          : ""}。
      </p>
    </div>
  {:else}
    <div class="summary">
      <strong>{groups.length.toLocaleString()} 组</strong>
      <span class="muted"
        >分析了 {result.images.toLocaleString()} 张图片，用时 {(result.elapsedMs / 1000).toFixed(1)} 秒{result.skipped >
        0
          ? `，${result.skipped} 张无法读取`
          : ""}{result.moreGroups > 0 ? `，另有 ${result.moreGroups} 组未列出` : ""}</span
      >
    </div>
    <div class="groups" {onscroll}>
      {#each groups.slice(0, shown) as files (files[0].id)}
        <article class="group">
          {#each files as f (f.id)}
            {@const on = marked.has(f.id)}
            <div class="tile" class:on>
              <button
                type="button"
                class="pic"
                onclick={() => toggle(files, f)}
                aria-pressed={on}
                aria-label="{on ? '取消勾选' : '勾选删除'} {f.path}"
              >
                <img use:thumb={f.id} alt="" />
                <span class="check" aria-hidden="true">{on ? "✓" : ""}</span>
                {#if !on}<span class="keep">保留</span>{/if}
              </button>
              <button type="button" class="name" title={f.path} onclick={() => onreveal(f.id)}
                >{name(f.path)}</button
              >
              <span class="meta"
                >{f.width}×{f.height} · {formatSize(f.size)} · {date(f.modifiedMs)}</span
              >
              {#if f.distance > 0}<span class="meta">差异 {f.distance}</span>{/if}
            </div>
          {/each}
        </article>
      {/each}
    </div>
    <footer>
      <span
        >已勾选 <strong>{markedFiles.length.toLocaleString()}</strong> 张，可释放
        <strong>{formatSize(markedSize)}</strong></span
      >
      <button
        type="button"
        class="link"
        onclick={() => marked.clear()}
        disabled={markedFiles.length === 0}>全部取消</button
      >
      <button
        type="button"
        class="primary"
        disabled={markedFiles.length === 0}
        onclick={() => ondelete(markedFiles.map((f) => f.id))}>删除勾选的图片…</button
      >
    </footer>
  {/if}
</section>

<style>
  .similar {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    border-radius: var(--radius);
    background: var(--panel);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }

  .title {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: 15px;
  }

  .title span,
  .muted {
    color: var(--ink-2);
    font-size: 12px;
  }

  .levels {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 7px;
    background: var(--field);
  }

  .levels button {
    height: 22px;
    padding: 0 10px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-2);
    font-size: 12px;
  }

  .levels button[aria-checked="true"] {
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }

  select {
    height: 26px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
    font: inherit;
  }

  button.primary,
  button.secondary {
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    font-weight: 500;
  }

  button.primary {
    background: var(--accent);
    color: var(--accent-ink);
  }

  button.secondary {
    background: var(--field);
  }

  button:disabled {
    opacity: 0.45;
  }

  .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 50%;
    background: var(--field);
    color: var(--ink-2);
  }

  .banner {
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--danger-bg);
    color: var(--danger);
  }

  .state {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    text-align: center;
  }

  .state p {
    margin: 0;
  }

  .progress {
    width: min(360px, 80%);
    height: 6px;
    margin: 6px 0;
    border-radius: 999px;
    background: var(--track);
    overflow: hidden;
  }

  .progress span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.2s;
  }

  .summary {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }

  .groups {
    flex: 1;
    min-height: 0;
    padding: 10px 12px;
    overflow-y: auto;
  }

  .group {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin: 0 0 12px;
    padding: 10px;
    border-radius: 12px;
    background: var(--panel-2);
  }

  .tile {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 160px;
    font-size: 11px;
  }

  .pic {
    position: relative;
    display: grid;
    place-items: center;
    width: 160px;
    height: 120px;
    padding: 0;
    border: 2px solid transparent;
    border-radius: 9px;
    background: var(--field);
    overflow: hidden;
    cursor: pointer;
  }

  .tile.on .pic {
    border-color: var(--danger);
  }

  .pic img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  .pic :global(img.broken) {
    visibility: hidden;
  }

  .check {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: 1.5px solid rgb(255 255 255 / 0.9);
    border-radius: 50%;
    background: rgb(0 0 0 / 0.25);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
  }

  .tile.on .check {
    border-color: var(--danger);
    background: var(--danger);
  }

  .keep {
    position: absolute;
    bottom: 6px;
    left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--good);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
  }

  .name {
    padding: 0;
    border: 0;
    background: none;
    overflow: hidden;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name:hover {
    color: var(--accent);
    text-decoration: underline;
  }

  .meta {
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-top: 1px solid var(--line);
    background: var(--panel-2);
  }

  footer > span {
    flex: 1;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
  }
</style>

<script lang="ts">
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import type { LiveEntry, ScanMode } from "$lib/api";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    path: string;
    mode: ScanMode | null;
    fallbackReason: string | null;
    processed: number;
    elapsedMs: number;
    /** 边扫边看：各顶层项目目前累计的大小，从大到小。只有普通遍历才有。 */
    live: LiveEntry[];
    liveTotal: number;
    oncancel: () => void;
  }

  let { path, mode, fallbackReason, processed, elapsedMs, live, liveTotal, oncancel }: Props =
    $props();

  /** 列出的顶层项目数。 */
  const SHOWN = 12;

  const rate = $derived(elapsedMs > 0 ? Math.round((processed / elapsedMs) * 1000) : 0);
  const shown = $derived(live.slice(0, SHOWN));
  const largest = $derived(live[0]?.size || 1);
  const rest = $derived(liveTotal - shown.reduce((sum, i) => sum + i.size, 0));
</script>

<main class="scan">
  <section class="card" class:wide={live.length > 0} aria-live="polite">
    <div class="spinner" aria-hidden="true"></div>
    <h1>正在扫描 {path}</h1>
    <p class="mode">
      {#if mode === "mft"}
        <span class="badge good">MFT 直读</span>
      {:else if mode === "walk"}
        <span class="badge">普通遍历</span>
      {:else}
        准备中…
      {/if}
    </p>
    <dl>
      <div>
        <dt>{mode === "mft" ? "已解析记录" : "已处理项目"}</dt>
        <dd>{processed.toLocaleString()}</dd>
      </div>
      <div>
        <dt>用时</dt>
        <dd>{(elapsedMs / 1000).toFixed(1)} 秒</dd>
      </div>
      <div>
        <dt>速度</dt>
        <dd>{rate.toLocaleString()}/秒</dd>
      </div>
    </dl>
    {#if fallbackReason}
      <p class="notice">未能使用 MFT 直读（{fallbackReason}），已改用普通遍历，会慢一些。</p>
    {/if}
    {#if live.length > 0}
      <!-- 边扫边看：顶层项目的大小随扫描增长，排名变化时平滑换位。 -->
      <div class="live" aria-label="目前发现的最大项目">
        <div class="live-head">
          <span>目前发现</span>
          <span>{formatSize(liveTotal)}</span>
        </div>
        <ul>
          {#each shown as item (item.name)}
            <li animate:flip={{ duration: 300, easing: cubicOut }}>
              <span class="icon" class:dir={item.isDir} aria-hidden="true"></span>
              <span class="name" title={item.name}>{item.name}</span>
              <span class="bar" aria-hidden="true"
                ><span style:width="{Math.max(1, (item.size / largest) * 100)}%"></span></span
              >
              <span class="size">{formatSize(item.size)}</span>
              <span class="pct">{formatPercent(item.size, liveTotal)}</span>
            </li>
          {/each}
        </ul>
        {#if live.length > SHOWN && rest > 0}
          <p class="rest">其余项目共 {formatSize(rest)}</p>
        {/if}
      </div>
    {/if}
    <button type="button" onclick={oncancel}>取消</button>
  </section>
</main>

<style>
  .scan {
    display: grid;
    place-items: center;
    box-sizing: border-box;
    min-height: 100%;
    padding: 24px;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    width: min(440px, 100%);
    transition: width 0.3s cubic-bezier(0.32, 0.72, 0, 1);
    padding: 32px 28px 24px;
    border-radius: 16px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    text-align: center;
  }

  .card.wide {
    width: min(620px, 100%);
  }

  .live {
    align-self: stretch;
    text-align: left;
  }

  .live-head {
    display: flex;
    justify-content: space-between;
    padding: 0 2px 6px;
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .live ul {
    margin: 0;
    padding: 4px;
    border-radius: 10px;
    background: var(--panel-2);
    list-style: none;
  }

  .live li {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) 120px 72px 46px;
    gap: 8px;
    align-items: center;
    height: 26px;
    padding: 0 8px;
    border-radius: 6px;
    font-size: 12px;
  }

  .live li:nth-child(even) {
    background: var(--stripe);
  }

  .icon {
    width: 12px;
    height: 10px;
    border-radius: 2px;
    background: var(--ink-3);
  }

  .icon.dir {
    background: var(--cat-app);
  }

  .live .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .live .bar {
    height: 5px;
    border-radius: 999px;
    background: var(--track);
    overflow: hidden;
  }

  .live .bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.4s cubic-bezier(0.32, 0.72, 0, 1);
  }

  .live .size,
  .live .pct {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .live .pct {
    color: var(--ink-2);
  }

  .rest {
    margin: 6px 2px 0;
    color: var(--ink-2);
    font-size: 11px;
  }

  /* macOS 风格的转圈指示器。 */
  .spinner {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: conic-gradient(from 0deg, transparent 0 15%, var(--accent));
    mask: radial-gradient(farthest-side, transparent calc(100% - 4px), #000 calc(100% - 4px));
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }

  h1 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    word-break: break-all;
  }

  .mode {
    margin: -6px 0 0;
    color: var(--ink-2);
  }

  .badge {
    padding: 3px 10px;
    border-radius: 999px;
    background: var(--track);
    font-size: 11px;
    font-weight: 600;
  }

  .badge.good {
    background: var(--good-bg);
    color: var(--good);
  }

  dl {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    width: 100%;
    margin: 4px 0 0;
    border-radius: 10px;
    background: var(--line);
    overflow: hidden;
  }

  dl div {
    padding: 10px 8px;
    background: var(--panel-2);
  }

  dt {
    color: var(--ink-2);
    font-size: 11px;
  }

  dd {
    margin: 2px 0 0;
    font-size: 15px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .notice {
    margin: 0;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--warn-bg);
    color: var(--warn);
    text-align: left;
  }

  button {
    height: 30px;
    padding: 0 22px;
    border: 0;
    border-radius: 8px;
    background: var(--field);
    font-weight: 500;
  }

  button:hover {
    background: var(--track);
  }
</style>

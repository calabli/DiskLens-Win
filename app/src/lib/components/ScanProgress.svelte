<script lang="ts">
  import type { ScanMode } from "$lib/api";

  interface Props {
    path: string;
    mode: ScanMode | null;
    fallbackReason: string | null;
    processed: number;
    elapsedMs: number;
    oncancel: () => void;
  }

  let { path, mode, fallbackReason, processed, elapsedMs, oncancel }: Props = $props();

  const rate = $derived(elapsedMs > 0 ? Math.round((processed / elapsedMs) * 1000) : 0);
</script>

<main class="scan">
  <section class="card" aria-live="polite">
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
    <div class="bar" role="progressbar" aria-label="扫描进度"><span></span></div>
    <dl>
      <div>
        <dt>{mode === "mft" ? "已解析记录" : "已处理条目"}</dt>
        <dd>{processed.toLocaleString()}</dd>
      </div>
      <div>
        <dt>用时</dt>
        <dd>{(elapsedMs / 1000).toFixed(1)} 秒</dd>
      </div>
      <div>
        <dt>速度</dt>
        <dd>{rate.toLocaleString()} 条/秒</dd>
      </div>
    </dl>
    {#if fallbackReason}
      <p class="notice">未能使用 MFT 直读（{fallbackReason}），已改用普通遍历，会慢一些。</p>
    {/if}
    <button type="button" onclick={oncancel}>取消</button>
  </section>
</main>

<style>
  .scan {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding: 24px;
  }

  .card {
    width: min(560px, 100%);
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 24px;
    border: 1px solid var(--line);
    border-radius: 14px;
    background: var(--panel);
  }

  h1 {
    margin: 0;
    font-size: 18px;
    word-break: break-all;
  }

  .mode {
    margin: 0;
  }

  .badge {
    padding: 3px 10px;
    border-radius: 999px;
    background: var(--track);
    font-size: 12px;
    font-weight: 600;
  }

  .badge.good {
    background: var(--good-bg);
    color: var(--good);
  }

  /* 总量未知，用来回移动的进度条表示仍在进行。 */
  .bar {
    position: relative;
    height: 8px;
    border-radius: 4px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    position: absolute;
    width: 30%;
    height: 100%;
    border-radius: 4px;
    background: var(--accent);
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    from {
      left: -30%;
    }
    to {
      left: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .bar span {
      animation: none;
      width: 100%;
      opacity: 0.4;
    }
  }

  dl {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin: 0;
  }

  dl div {
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--bg);
  }

  dt {
    color: var(--ink-2);
    font-size: 12px;
  }

  dd {
    margin: 2px 0 0;
    font-size: 16px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .notice {
    margin: 0;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--warn-bg);
    color: var(--warn);
  }

  button {
    align-self: flex-start;
    height: 40px;
    padding: 0 20px;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--panel);
    font: inherit;
    font-weight: 600;
    color: inherit;
    cursor: pointer;
  }
</style>

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
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    width: min(440px, 100%);
    padding: 32px 28px 24px;
    border-radius: 16px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    text-align: center;
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

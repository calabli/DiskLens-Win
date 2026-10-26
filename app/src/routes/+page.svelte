<script lang="ts">
  import {
    cachedScans,
    cancelScan,
    openCached,
    startScan,
    type CachedScan,
    type LiveEntry,
    type ScanMode,
    type ScanSummary,
  } from "$lib/api";
  import Explorer from "$lib/components/Explorer.svelte";
  import Home from "$lib/components/Home.svelte";
  import ScanProgress from "$lib/components/ScanProgress.svelte";

  type Phase = "starting" | "home" | "scanning" | "result";

  let phase = $state<Phase>("starting");
  let cached = $state<CachedScan[]>([]);
  let scanPath = $state("");
  let mode = $state<ScanMode | null>(null);
  let fallbackReason = $state<string | null>(null);
  let processed = $state(0);
  let elapsedMs = $state(0);
  let live = $state<LiveEntry[]>([]);
  let liveTotal = $state(0);
  let summary = $state<ScanSummary | null>(null);
  let error = $state("");
  /** 每次扫描递增；用于让结果页在重新扫描后整个重建。 */
  let scanSeq = $state(0);

  async function scan(path: string) {
    scanPath = path;
    mode = null;
    fallbackReason = null;
    processed = 0;
    elapsedMs = 0;
    live = [];
    liveTotal = 0;
    error = "";
    const previous = phase;
    phase = "scanning";
    try {
      summary = await startScan(path, (e) => {
        if (e.kind === "started") {
          mode = e.mode;
          fallbackReason = e.fallbackReason;
        } else if (e.kind === "live") {
          live = e.items;
          liveTotal = e.totalSize;
        } else {
          processed = e.processed;
          elapsedMs = e.elapsedMs;
        }
      });
      scanSeq++;
      phase = "result";
    } catch (e) {
      const message = String(e);
      if (message !== "已取消") error = message;
      phase = previous === "result" && summary ? "result" : "home";
    }
  }

  async function cancel() {
    await cancelScan();
  }

  /** 打开某个路径上次的扫描结果。 */
  async function open(root: string) {
    try {
      summary = await openCached(root);
      scanSeq++;
      phase = "result";
    } catch (e) {
      error = String(e);
      phase = "home";
    }
  }

  function home() {
    phase = "home";
    cachedScans()
      .then((c) => (cached = c))
      .catch(() => {});
  }

  // 启动时直接显示最近一次的扫描结果；没有缓存时停在首页。
  $effect(() => {
    cachedScans()
      .then((c) => {
        cached = c;
        if (c.length > 0) open(c[0].root);
        else phase = "home";
      })
      .catch(() => (phase = "home"));
  });
</script>

{#if error}
  <p class="error" role="alert">
    {error}
    <button type="button" onclick={() => (error = "")}>关闭</button>
  </p>
{/if}

{#if phase === "home"}
  <Home {cached} onscan={scan} onopen={open} />
{:else if phase === "scanning"}
  <ScanProgress
    path={scanPath}
    {mode}
    {fallbackReason}
    {processed}
    {elapsedMs}
    {live}
    {liveTotal}
    oncancel={cancel}
  />
{:else if summary}
  {#key scanSeq}
    <Explorer {summary} onrescan={() => summary && scan(summary.root)} onhome={home} />
  {/key}
{/if}

<style>
  .error {
    position: fixed;
    top: 14px;
    left: 50%;
    z-index: 10;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 0;
    padding: 10px 10px 10px 16px;
    border-radius: 12px;
    background: var(--panel);
    color: var(--danger);
    box-shadow: var(--shadow-lift);
  }

  .error button {
    padding: 4px 10px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
  }
</style>

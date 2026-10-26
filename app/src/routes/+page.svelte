<script lang="ts">
  import { cancelScan, startScan, type ScanMode, type ScanSummary } from "$lib/api";
  import Explorer from "$lib/components/Explorer.svelte";
  import Home from "$lib/components/Home.svelte";
  import ScanProgress from "$lib/components/ScanProgress.svelte";

  type Phase = "home" | "scanning" | "result";

  let phase = $state<Phase>("home");
  let scanPath = $state("");
  let mode = $state<ScanMode | null>(null);
  let fallbackReason = $state<string | null>(null);
  let processed = $state(0);
  let elapsedMs = $state(0);
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
    error = "";
    const previous = phase;
    phase = "scanning";
    try {
      summary = await startScan(path, (e) => {
        if (e.kind === "started") {
          mode = e.mode;
          fallbackReason = e.fallbackReason;
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
</script>

{#if error}
  <p class="error" role="alert">
    {error}
    <button type="button" onclick={() => (error = "")}>关闭</button>
  </p>
{/if}

{#if phase === "home"}
  <Home onscan={scan} />
{:else if phase === "scanning"}
  <ScanProgress path={scanPath} {mode} {fallbackReason} {processed} {elapsedMs} oncancel={cancel} />
{:else if summary}
  {#key scanSeq}
    <Explorer
      {summary}
      onrescan={() => summary && scan(summary.root)}
      onhome={() => (phase = "home")}
    />
  {/key}
{/if}

<style>
  :global(:root) {
    --bg: #f4f3ef;
    --panel: #ffffff;
    --ink: #1c1c1a;
    --ink-2: #5f5e57;
    --ink-3: #8a877e;
    --line: #e4e2dc;
    --line-strong: #d9d6ce;
    --track: #eceae4;
    --hover: #f2f1ec;
    --selected: #eaf0fc;
    --accent: #2f6feb;
    --good: #1d5c45;
    --good-bg: #e3f1ea;
    --warn: #7a4b0c;
    --warn-bg: #fbefd9;
    --danger: #8a2a1f;
  }

  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--ink);
    font:
      13px/1.4 "Segoe UI",
      "Microsoft YaHei UI",
      system-ui,
      sans-serif;
  }

  :global(html),
  :global(body) {
    height: 100%;
  }

  .error {
    position: fixed;
    top: 12px;
    left: 50%;
    z-index: 10;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0;
    padding: 10px 14px;
    border-radius: 10px;
    background: #f8e1de;
    color: var(--danger);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.12);
  }

  .error button {
    border: 0;
    background: none;
    font: inherit;
    font-weight: 600;
    color: inherit;
    cursor: pointer;
  }
</style>

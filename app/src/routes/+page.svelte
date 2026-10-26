<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { formatPercent, formatSize } from "$lib/format";

  interface Entry {
    name: string;
    size: number;
    allocated: number;
    files: number;
    isDir: boolean;
  }

  interface ScanSummary {
    root: string;
    mode: "mft" | "walk";
    fallbackReason: string | null;
    totalSize: number;
    totalAllocated: number;
    fileCount: number;
    nodeCount: number;
    skipped: number;
    elapsedMs: number;
    children: Entry[];
  }

  let path = $state("D:\\");
  let scanning = $state(false);
  let error = $state("");
  let summary = $state<ScanSummary | null>(null);

  const largest = $derived(summary?.children[0]?.size ?? 1);

  async function scan(event: SubmitEvent) {
    event.preventDefault();
    scanning = true;
    error = "";
    try {
      summary = await invoke<ScanSummary>("scan_folder", { path });
    } catch (e) {
      error = String(e);
      summary = null;
    } finally {
      scanning = false;
    }
  }
</script>

<main>
  <header>
    <h1>DiskLens</h1>
    <form onsubmit={scan}>
      <label for="path">文件夹</label>
      <input id="path" bind:value={path} spellcheck="false" autocomplete="off" />
      <button type="submit" disabled={scanning}>{scanning ? "扫描中…" : "扫描"}</button>
    </form>
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if summary}
    <p class="summary">
      {[
        summary.root,
        summary.mode === "mft" ? "MFT 直读" : "普通遍历",
        formatSize(summary.totalSize),
        `占用 ${formatSize(summary.totalAllocated)}`,
        `${summary.fileCount.toLocaleString()} 个文件`,
        `用时 ${(summary.elapsedMs / 1000).toFixed(1)} 秒`,
        summary.skipped > 0 ? `${summary.skipped} 个目录无权读取` : null,
      ]
        .filter(Boolean)
        .join(" · ")}
    </p>
    {#if summary.fallbackReason}
      <p class="notice">未能使用 MFT 直读（{summary.fallbackReason}），已改用普通遍历。</p>
    {/if}
    <ul>
      {#each summary.children as child (child.name)}
        <li>
          <span class="name" class:dir={child.isDir}>{child.name}</span>
          <span class="size">{formatSize(child.size)}</span>
          <span class="bar"><span style:width="{(child.size / largest) * 100}%"></span></span>
          <span class="pct">{formatPercent(child.size, summary.totalSize)}</span>
        </li>
      {/each}
    </ul>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    background: #f4f3ef;
    color: #1c1c1a;
    font:
      13px/1.4 "Segoe UI",
      "Microsoft YaHei UI",
      system-ui,
      sans-serif;
  }

  main {
    max-width: 960px;
    margin: 0 auto;
    padding: 16px;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 24px;
  }

  h1 {
    margin: 0;
    font-size: 18px;
  }

  form {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 36px;
    padding: 0 10px;
    border: 1px solid #d9d6ce;
    border-radius: 8px;
    font: inherit;
  }

  button {
    height: 36px;
    padding: 0 16px;
    border: 0;
    border-radius: 8px;
    background: #2f6feb;
    color: #fff;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  button:disabled {
    background: #b8b4aa;
    cursor: default;
  }

  .error {
    color: #8a2a1f;
  }

  .summary {
    color: #5f5e57;
  }

  .notice {
    padding: 8px 12px;
    border-radius: 8px;
    background: #fbefd9;
    color: #7a4b0c;
  }

  ul {
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: #fff;
    border: 1px solid #e4e2dc;
    border-radius: 12px;
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 80px 120px 56px;
    gap: 12px;
    align-items: center;
    padding: 6px 14px;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name::before {
    content: "";
    display: inline-block;
    width: 10px;
    height: 10px;
    margin-right: 8px;
    border-radius: 2px;
    background: #c9c4b8;
  }

  .name.dir::before {
    background: #3a6ac8;
  }

  .size,
  .pct {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .pct {
    color: #5f5e57;
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: #eceae4;
  }

  .bar span {
    display: block;
    height: 6px;
    border-radius: 3px;
    background: #3a6ac8;
  }
</style>

<script lang="ts">
  import { listVolumes, type Volume } from "$lib/api";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    onscan: (path: string) => void;
  }

  let { onscan }: Props = $props();

  let volumes = $state<Volume[]>([]);
  let error = $state("");
  let folder = $state("D:\\");

  $effect(() => {
    listVolumes()
      .then((v) => (volumes = v))
      .catch((e) => (error = String(e)));
  });

  function scanFolder(event: SubmitEvent) {
    event.preventDefault();
    if (folder.trim()) onscan(folder.trim());
  }
</script>

<main class="home">
  <h1>选择要分析的磁盘</h1>
  <p class="hint">
    以管理员身份运行时，NTFS 磁盘会直接读取 MFT，几秒内完成；否则改用逐个目录遍历，会慢一些。
  </p>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <ul class="volumes">
    {#each volumes as v (v.root)}
      {@const used = v.total - v.free}
      <li>
        <button type="button" class="volume" onclick={() => onscan(v.root)}>
          <span class="top">
            <span class="letter">{v.root.slice(0, 2)}</span>
            <span class="label">{v.label || "本地磁盘"}</span>
            <span class="fs">{v.fileSystem}</span>
          </span>
          <span class="bar" aria-hidden="true"
            ><span style:width={formatPercent(used, v.total)}></span></span
          >
          <span class="usage">
            已用 {formatSize(used)} / {formatSize(v.total)} · 可用 {formatSize(v.free)}
          </span>
        </button>
      </li>
    {/each}
  </ul>

  <form class="folder" onsubmit={scanFolder}>
    <label for="folder">或者扫描某个文件夹</label>
    <div class="row">
      <input id="folder" bind:value={folder} spellcheck="false" autocomplete="off" />
      <button type="submit">扫描</button>
    </div>
  </form>
</main>

<style>
  .home {
    max-width: 880px;
    margin: 0 auto;
    padding: 40px 24px;
  }

  h1 {
    margin: 0;
    font-size: 22px;
  }

  .hint {
    margin: 8px 0 24px;
    color: var(--ink-2);
  }

  .error {
    color: var(--danger);
  }

  .volumes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
    margin: 0 0 32px;
    padding: 0;
    list-style: none;
  }

  .volume {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--panel);
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  .volume:hover {
    border-color: var(--accent);
  }

  .volume:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .top {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .letter {
    font-size: 20px;
    font-weight: 700;
  }

  .label {
    flex: 1;
  }

  .fs {
    color: var(--ink-2);
    font-size: 12px;
  }

  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .usage {
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .folder label {
    display: block;
    margin-bottom: 8px;
    font-weight: 600;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 40px;
    padding: 0 12px;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--panel);
    font: inherit;
    color: inherit;
  }

  .row button {
    height: 40px;
    padding: 0 20px;
    border: 0;
    border-radius: 10px;
    background: var(--accent);
    color: #fff;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
</style>

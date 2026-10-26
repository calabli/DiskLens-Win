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
  <header>
    <h1>DiskLens</h1>
    <p>选择一个磁盘，看看空间都去哪儿了。</p>
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <ul class="volumes">
    {#each volumes as v (v.root)}
      {@const used = v.total - v.free}
      <li>
        <button type="button" class="volume" onclick={() => onscan(v.root)}>
          <svg class="disk" viewBox="0 0 32 32" width="34" height="34" aria-hidden="true">
            <rect x="3" y="7" width="26" height="18" rx="4" fill="var(--track)"></rect>
            <rect x="3" y="7" width="26" height="18" rx="4" fill="none" stroke="var(--line-strong)"
            ></rect>
            <circle cx="23.5" cy="16" r="1.6" fill="var(--good)"></circle>
          </svg>
          <span class="meta">
            <span class="name"
              >{v.label || "本地磁盘"} <span class="letter">{v.root.slice(0, 2)}</span></span
            >
            <span class="usage">{formatSize(v.free)} 可用，共 {formatSize(v.total)}</span>
            <span class="bar" aria-hidden="true"
              ><span style:width={formatPercent(used, v.total)}></span></span
            >
          </span>
          <span class="fs">{v.fileSystem}</span>
        </button>
      </li>
    {/each}
  </ul>

  <form class="folder" onsubmit={scanFolder}>
    <label for="folder">扫描某个文件夹</label>
    <div class="row">
      <input id="folder" bind:value={folder} spellcheck="false" autocomplete="off" />
      <button type="submit">扫描</button>
    </div>
    <p class="hint">以管理员身份运行时，NTFS 磁盘会直接读取 MFT，几秒内完成。</p>
  </form>
</main>

<style>
  .home {
    max-width: 720px;
    margin: 0 auto;
    padding: 56px 24px;
  }

  header h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  header p {
    margin: 6px 0 28px;
    color: var(--ink-2);
    font-size: 15px;
  }

  .error {
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--danger-bg);
    color: var(--danger);
  }

  .volumes {
    display: flex;
    flex-direction: column;
    margin: 0 0 32px;
    padding: 0;
    list-style: none;
    border-radius: var(--radius);
    background: var(--panel);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  li + li .volume {
    border-top: 1px solid var(--line);
  }

  .volume {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 14px 18px;
    border: 0;
    background: transparent;
    text-align: left;
    transition: background-color 0.12s;
  }

  .volume:hover {
    background: var(--hover);
  }

  .meta {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .name {
    font-size: 14px;
    font-weight: 600;
  }

  .letter {
    color: var(--ink-2);
    font-weight: 400;
  }

  .usage {
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 6px;
    margin-top: 2px;
    border-radius: 999px;
    background: var(--track);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
  }

  .fs {
    color: var(--ink-3);
    font-size: 11px;
  }

  .folder label {
    display: block;
    margin: 0 0 8px 4px;
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 600;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 12px;
    border: 0;
    border-radius: 8px;
    background: var(--panel);
    box-shadow: var(--shadow);
    outline: none;
  }

  input:focus {
    box-shadow:
      var(--shadow),
      0 0 0 3px var(--accent-soft);
  }

  .row button {
    height: 34px;
    padding: 0 18px;
    border: 0;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 600;
  }

  .row button:active {
    filter: brightness(0.92);
  }

  .hint {
    margin: 10px 4px 0;
    color: var(--ink-3);
    font-size: 12px;
  }
</style>

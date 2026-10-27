<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { listVolumes, moveFolder, type MoveResult, type Volume } from "$lib/api";
  import { formatSize } from "$lib/format";

  interface Props {
    id: number;
    name: string;
    path: string;
    size: number;
    onmoved: (r: MoveResult) => void;
    onclose: () => void;
  }

  let { id, name, path, size, onmoved, onclose }: Props = $props();

  /** 目标磁盘至少要比文件夹多留这么多空间（与 Rust 侧一致）。 */
  const HEADROOM = 64 << 20;

  const from = $derived(path.slice(0, 2).toUpperCase());
  let volumes = $state<Volume[]>([]);
  let drive = $state("");
  let folder = $state("");
  let moving = $state(false);
  let done = $state(0);
  let total = $state(0);
  let error = $state("");

  const usable = $derived(
    volumes.filter((v) => v.root.slice(0, 2).toUpperCase() !== from && v.fileSystem === "NTFS"),
  );
  const target = $derived(`${folder.replace(/\\+$/, "")}\\${name}`);

  $effect(() => {
    listVolumes()
      .then((v) => {
        volumes = v;
        const best = usable
          .filter((x) => x.free >= size + HEADROOM)
          .sort((a, b) => b.free - a.free)[0];
        if (best) pick(best);
      })
      .catch((e) => (error = String(e)));
  });

  function pick(v: Volume) {
    drive = v.root;
    folder = `${v.root.slice(0, 2)}\\DiskLens 移出`;
  }

  async function start() {
    moving = true;
    error = "";
    try {
      const r = await moveFolder(id, folder, (d, t) => {
        done = d;
        total = t;
      });
      onmoved(r);
    } catch (e) {
      error = String(e);
      moving = false;
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !moving) onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="backdrop" transition:fade={{ duration: 150 }} role="presentation"></div>
<div
  class="sheet"
  role="dialog"
  aria-modal="true"
  aria-labelledby="move-title"
  transition:fly={{ y: -24, duration: 220, easing: cubicOut }}
>
  <h2 id="move-title">把“{name}”移到其他磁盘</h2>
  <p>
    文件夹会被复制到新位置、核对无误后从这里删除，原位置留下一个指向新位置的<strong>连接点</strong
    >： 依赖原路径的程序照常工作，这个磁盘释放 <strong>{formatSize(size)}</strong>。
  </p>
  <p class="muted">文件有正在使用的会直接停止，什么也不改；中途出错会自动撤回。</p>

  <fieldset disabled={moving}>
    <legend>移到哪个磁盘</legend>
    {#each usable as v (v.root)}
      {@const enough = v.free >= size + HEADROOM}
      <label class="drive" class:off={!enough}>
        <input
          type="radio"
          name="drive"
          checked={drive === v.root}
          disabled={!enough}
          onchange={() => pick(v)}
        />
        <span>{v.label || "本地磁盘"} {v.root.slice(0, 2)}</span>
        <span class="muted">{formatSize(v.free)} 可用{enough ? "" : "，不够"}</span>
      </label>
    {:else}
      <p class="muted">没有其他可用的 NTFS 磁盘。</p>
    {/each}
    {#if drive}
      <label class="folder">
        <span>放在这个文件夹里</span>
        <input bind:value={folder} spellcheck="false" />
      </label>
      <p class="muted target">移动后的位置：{target}</p>
    {/if}
  </fieldset>

  {#if moving}
    <div class="progress" role="progressbar">
      <span style:width="{total > 0 ? (done / total) * 100 : 0}%"></span>
    </div>
    <p class="muted">正在复制 {formatSize(done)} / {formatSize(total)}，请不要关闭程序…</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <button type="button" class="secondary" onclick={onclose} disabled={moving}>取消</button>
    <button
      type="button"
      class="primary"
      onclick={start}
      disabled={moving || !drive || !folder.trim()}>{moving ? "正在移动…" : "移动"}</button
    >
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: rgb(0 0 0 / 0.25);
  }

  .sheet {
    position: fixed;
    top: 64px;
    left: 50%;
    z-index: 21;
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(460px, calc(100vw - 32px));
    padding: 22px 22px 18px;
    border-radius: 14px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    transform: translateX(-50%);
  }

  h2 {
    margin: 0;
    font-size: 15px;
    text-align: center;
    word-break: break-all;
  }

  p {
    margin: 0;
    line-height: 1.6;
  }

  .muted {
    color: var(--ink-2);
    font-size: 12px;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    margin-bottom: 4px;
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 600;
  }

  .drive {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--field);
  }

  .drive .muted {
    margin-left: auto;
  }

  .drive.off {
    opacity: 0.5;
  }

  .folder {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
    color: var(--ink-2);
    font-size: 12px;
  }

  .folder input {
    height: 28px;
    padding: 0 10px;
    border: 0;
    border-radius: 7px;
    background: var(--field);
    color: var(--ink);
    outline: none;
    user-select: text;
  }

  .target {
    word-break: break-all;
  }

  .progress {
    height: 6px;
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

  .error {
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--danger-bg);
    color: var(--danger);
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  .actions button {
    flex: 1;
    height: 30px;
    border: 0;
    border-radius: 8px;
    font-weight: 500;
  }

  .secondary {
    background: var(--field);
  }

  .primary {
    background: var(--accent);
    color: var(--accent-ink);
  }

  button:disabled {
    opacity: 0.45;
  }
</style>

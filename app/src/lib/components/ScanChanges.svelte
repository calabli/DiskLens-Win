<script lang="ts">
  import { compareScan, setBaseline, type ChangeItem, type Comparison } from "$lib/api";
  import { formatAgo, formatSize } from "$lib/format";

  interface Props {
    /** 数据版本；删除或刷新后递增，重新对比。 */
    version: number;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { version, onreveal, onclose }: Props = $props();

  let result = $state<Comparison | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");

  function load() {
    loading = true;
    compareScan()
      .then((r) => {
        result = r;
        error = "";
      })
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  }

  $effect(() => {
    void version;
    load();
  });

  async function rebase() {
    saving = true;
    try {
      await setBaseline();
      load();
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  /** 带正负号的大小，例如 “+1.2 GB”。 */
  function signed(delta: number) {
    if (delta === 0) return "0 B";
    return `${delta > 0 ? "+" : "−"}${formatSize(Math.abs(delta))}`;
  }

  function name(path: string) {
    return path.slice(path.lastIndexOf("\\") + 1) || path;
  }

  function folder(path: string) {
    return path.slice(0, path.lastIndexOf("\\") + 1);
  }

  const STATUS: Record<ChangeItem["status"], string> = {
    new: "新增",
    removed: "已不在",
    changed: "",
  };
</script>

{#snippet list(items: ChangeItem[], empty: string)}
  {#if items.length === 0}
    <p class="muted">{empty}</p>
  {:else}
    <ul>
      {#each items as c (c.path)}
        {@const delta = c.after - c.before}
        <li>
          <div class="text">
            <span class="name" title={c.path}
              >{name(c.path)}{#if STATUS[c.status]}<em class="tag {c.status}">{STATUS[c.status]}</em
                >{/if}</span
            >
            <span class="path" title={c.path}>{folder(c.path)}</span>
          </div>
          <span class="was">{formatSize(c.before)} → {formatSize(c.after)}</span>
          <span class="delta" class:up={delta > 0} class:down={delta < 0}>{signed(delta)}</span>
          {#if c.id != null}
            <button type="button" class="link" onclick={() => onreveal(c.id!)}>定位</button>
          {:else}
            <span></span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
{/snippet}

<section class="changes" aria-label="变化对比">
  <header>
    <div class="title">
      <h2>变化对比</h2>
      <span>和对比基准（上一次完整扫描）相比，哪些文件夹和文件变大了、变小了。</span>
    </div>
    <button type="button" class="secondary" disabled={saving || loading} onclick={rebase}
      >{saving ? "正在保存…" : "把现在设为对比基准"}</button
    >
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

  <div class="scroll">
    {#if loading && !result}
      <p class="muted">正在对比…</p>
    {:else if result && result.baseScannedAt == null}
      <div class="empty">
        <p>还没有可以对比的上一次扫描。</p>
        <p class="muted">
          下次完整扫描（换一个位置扫描，或按住 Shift
          点刷新）后，就能和这次的结果对比；也可以现在点“把现在设为对比基准”，过一段时间再回来看变化。
        </p>
      </div>
    {:else if result}
      {@const delta = result.afterSize - result.beforeSize}
      <div class="summary">
        <div class="big">
          <span class="delta" class:up={delta > 0} class:down={delta < 0}>{signed(delta)}</span>
          <span class="muted"
            >{formatSize(result.beforeSize)} → {formatSize(result.afterSize)}，文件数 {result.beforeFiles.toLocaleString()}
            → {result.afterFiles.toLocaleString()}</span
          >
        </div>
        <p class="muted">
          对比基准：{formatAgo(result.baseScannedAt!)}的扫描结果（{new Date(
            result.baseScannedAt!,
          ).toLocaleString("zh-CN")}）
        </p>
      </div>

      <h3>变大的 <span>{result.grown.length > 0 ? `${result.grown.length} 处` : ""}</span></h3>
      {@render list(result.grown, "没有明显变大的地方。")}

      <h3>变小的 <span>{result.shrunk.length > 0 ? `${result.shrunk.length} 处` : ""}</span></h3>
      {@render list(result.shrunk, "没有明显变小的地方。")}

      <p class="note muted">
        只列出变化较大的位置。一处变化如果主要发生在某个子文件夹里，就只列那个子文件夹，不再重复列它的上级。
      </p>
    {/if}
  </div>
</section>

<style>
  .changes {
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
    gap: 12px;
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

  .secondary {
    flex: none;
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    background: var(--field);
    color: var(--ink);
    font-weight: 500;
  }

  .secondary:disabled {
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
    background: var(--warn-bg);
    color: var(--warn);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 16px 18px;
  }

  .empty p {
    margin: 0 0 6px;
    max-width: 640px;
    line-height: 1.6;
  }

  .summary {
    padding: 14px 16px;
    border-radius: 12px;
    background: var(--stripe);
  }

  .summary p {
    margin: 6px 0 0;
  }

  .big {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px 14px;
  }

  .big .delta {
    font-size: 26px;
    font-weight: 600;
  }

  h3 {
    margin: 20px 0 6px;
    font-size: 13px;
  }

  h3 span {
    margin-left: 4px;
    color: var(--ink-2);
    font-weight: 400;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 90px 44px;
    align-items: center;
    gap: 12px;
    padding: 5px 0;
    border-top: 1px solid var(--line);
  }

  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .path {
    color: var(--ink-2);
    font-size: 11px;
  }

  .tag {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 999px;
    font-size: 10.5px;
    font-style: normal;
    font-weight: 600;
  }

  .tag.new {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .tag.removed {
    background: var(--track);
    color: var(--ink-2);
  }

  .was {
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .delta {
    text-align: right;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .delta.up {
    color: var(--danger);
  }

  .delta.down {
    color: var(--good);
  }

  .link {
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--accent);
  }

  .link:hover {
    background: var(--accent-soft);
  }

  .note {
    margin-top: 16px;
  }
</style>

<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { search, type SearchHit } from "$lib/api";
  import { formatDate, formatSize } from "$lib/format";

  interface Props {
    rootId: number;
    rootName: string;
    /** 已经删除的项目，从结果里去掉。 */
    removed: Set<number>;
    ondelete: (ids: number[]) => void;
    /** 把勾选的项目加入收集篮，稍后和别处的项目一起删除。 */
    onbasket: (ids: number[]) => void;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { rootId, rootName, removed, ondelete, onbasket, onreveal, onclose }: Props = $props();

  type Mode = "largest" | "unused";

  const COUNTS = [100, 500, 1000];
  const SIZES = [
    { label: "全部大小", query: "" },
    { label: "≥ 10 MB", query: "size:>=10mb" },
    { label: "≥ 100 MB", query: "size:>=100mb" },
    { label: "≥ 1 GB", query: "size:>=1gb" },
  ];
  const AGES = [
    { label: "半年", query: "6m" },
    { label: "1 年", query: "1y" },
    { label: "2 年", query: "2y" },
    { label: "3 年", query: "3y" },
  ];

  let mode = $state<Mode>("largest");
  let count = $state(100);
  let size = $state("size:>=10mb");
  let age = $state("1y");
  /** 按访问时间还是修改时间判断“没用过”。 */
  let by = $state<"da" | "dm">("da");
  let running = $state(false);
  let hits = $state<SearchHit[] | null>(null);
  let error = $state("");
  const marked = new SvelteSet<number>();

  const query = $derived(
    ["file:", size, mode === "unused" ? `${by}:<${age}` : ""].filter(Boolean).join(" "),
  );
  const shown = $derived((hits ?? []).filter((h) => !removed.has(h.id)));
  const markedSize = $derived(
    shown.filter((h) => marked.has(h.id)).reduce((sum, h) => sum + h.size, 0),
  );
  const totalSize = $derived(shown.reduce((sum, h) => sum + h.size, 0));

  async function run() {
    running = true;
    error = "";
    marked.clear();
    try {
      hits = await search(rootId, query, count);
    } catch (e) {
      error = String(e);
    } finally {
      running = false;
    }
  }

  // 条件一变就重新查（在内存里查，很快）。
  $effect(() => {
    void [query, count, rootId];
    untrack(run);
  });

  function toggle(id: number) {
    if (marked.has(id)) marked.delete(id);
    else marked.add(id);
  }

  function split(path: string) {
    const i = path.lastIndexOf("\\");
    return { dir: path.slice(0, i + 1), name: path.slice(i + 1) };
  }
</script>

<section class="rank" aria-label="排行">
  <header>
    <div class="title">
      <h2>排行</h2>
      <span>{rootName} 中{mode === "largest" ? "最大的文件" : "很久没用过的大文件"}</span>
    </div>
    <div class="seg" role="radiogroup" aria-label="排行类型">
      <button
        type="button"
        role="radio"
        aria-checked={mode === "largest"}
        onclick={() => (mode = "largest")}>最大的文件</button
      >
      <button
        type="button"
        role="radio"
        aria-checked={mode === "unused"}
        onclick={() => (mode = "unused")}>长期未使用</button
      >
    </div>
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

  <div class="controls">
    {#if mode === "unused"}
      <label
        >超过
        <select bind:value={age}>
          {#each AGES as a (a.query)}<option value={a.query}>{a.label}</option>{/each}
        </select>
        没有
        <select bind:value={by}>
          <option value="da">访问</option>
          <option value="dm">修改</option>
        </select>
      </label>
    {/if}
    <label
      >大小
      <select bind:value={size}>
        {#each SIZES as s (s.query)}<option value={s.query}>{s.label}</option>{/each}
      </select>
    </label>
    <label
      >最多
      <select bind:value={count}>
        {#each COUNTS as c (c)}<option value={c}>{c} 个</option>{/each}
      </select>
    </label>
    <span class="query" title="对应的搜索条件，可以在搜索框里使用">{query}</span>
  </div>
  {#if mode === "unused" && by === "da"}
    <p class="note">
      Windows 默认可能不及时更新“访问时间”。如果结果看起来不对，可以改按修改时间判断。
    </p>
  {/if}

  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}

  {#if hits && shown.length === 0 && !running}
    <div class="state"><p>没有符合条件的文件。</p></div>
  {:else}
    <ol class="list" aria-busy={running}>
      {#each shown as h, i (h.id)}
        {@const p = split(h.path)}
        <li class:on={marked.has(h.id)}>
          <input
            type="checkbox"
            checked={marked.has(h.id)}
            onchange={() => toggle(h.id)}
            aria-label="删除 {h.path}"
          />
          <span class="no">{i + 1}</span>
          <button type="button" class="path" title="在主界面中定位" onclick={() => onreveal(h.id)}
            ><span class="dir">{p.dir}</span><span>{p.name}</span></button
          >
          <span class="date" title="修改 {formatDate(h.modified)}，访问 {formatDate(h.accessed)}"
            >{formatDate(mode === "unused" && by === "da" ? h.accessed : h.modified)}</span
          >
          <span class="size">{formatSize(h.size)}</span>
        </li>
      {/each}
    </ol>
  {/if}

  <footer>
    <span
      >共 {shown.length.toLocaleString()} 个，{formatSize(totalSize)}；已勾选
      <strong>{marked.size}</strong> 个，<strong>{formatSize(markedSize)}</strong></span
    >
    <button
      type="button"
      class="secondary"
      disabled={marked.size === 0}
      onclick={() => onbasket([...marked])}>加入收集篮</button
    >
    <button
      type="button"
      class="primary"
      disabled={marked.size === 0}
      onclick={() => ondelete([...marked])}>删除勾选的文件…</button
    >
  </footer>
</section>

<style>
  .rank {
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
  .note {
    color: var(--ink-2);
    font-size: 12px;
  }

  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 7px;
    background: var(--field);
  }

  .seg button {
    height: 24px;
    padding: 0 12px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-2);
  }

  .seg button[aria-checked="true"] {
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
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

  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
    padding: 10px 16px 4px;
    font-size: 12px;
  }

  .controls label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ink-2);
  }

  select {
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
    font: inherit;
  }

  .query {
    margin-left: auto;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink-2);
    font-family: ui-monospace, Consolas, monospace;
    user-select: text;
  }

  .note {
    margin: 4px 16px 0;
  }

  .banner {
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--danger-bg);
    color: var(--danger);
  }

  .state {
    display: grid;
    flex: 1;
    place-items: center;
    color: var(--ink-2);
  }

  .list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 6px 10px;
    overflow-y: auto;
    list-style: none;
  }

  .list[aria-busy="true"] {
    opacity: 0.6;
  }

  li {
    display: grid;
    grid-template-columns: 18px 36px minmax(0, 1fr) 84px 76px;
    gap: 8px;
    align-items: center;
    padding: 4px 8px;
    border-radius: 6px;
    font-size: 12px;
  }

  li:nth-child(even) {
    background: var(--stripe);
  }

  li:hover {
    background: var(--hover);
  }

  li.on {
    background: var(--danger-bg);
  }

  input[type="checkbox"] {
    margin: 0;
    accent-color: var(--danger);
  }

  .no {
    color: var(--ink-3);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .path {
    display: flex;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
    white-space: nowrap;
  }

  .path:hover span:last-child {
    color: var(--accent);
    text-decoration: underline;
  }

  .path .dir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--ink-2);
  }

  .path span:last-child {
    flex: none;
    max-width: 70%;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .date,
  .size {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .date {
    color: var(--ink-2);
  }

  .size {
    font-weight: 600;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-top: 1px solid var(--line);
    background: var(--panel-2);
  }

  footer > span {
    flex: 1;
  }

  .primary,
  .secondary {
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 500;
  }

  .secondary {
    background: var(--field);
    color: var(--ink);
  }

  .secondary:disabled {
    opacity: 0.45;
  }

  .primary:disabled {
    opacity: 0.45;
  }
</style>

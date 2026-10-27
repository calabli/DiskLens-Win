<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import {
    cancelIssues,
    findIssues,
    type Finding,
    type FindingKind,
    type FindResult,
  } from "$lib/api";

  interface Props {
    rootId: number;
    rootName: string;
    /** 已经删除的项目，从结果里去掉。 */
    removed: Set<number>;
    ondelete: (ids: number[]) => void;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { rootId, rootName, removed, ondelete, onreveal, onclose }: Props = $props();

  const SECTIONS: { kind: FindingKind; title: string; note: string }[] = [
    {
      kind: "brokenLink",
      title: "失效的链接",
      note: "符号链接或连接点指向的位置已经不存在。删除只去掉链接本身。",
    },
    {
      kind: "brokenShortcut",
      title: "失效的快捷方式",
      note: "快捷方式指向的文件或文件夹已经不存在（目标所在磁盘未连接的不算）。",
    },
    {
      kind: "emptyDir",
      title: "空文件夹",
      note: "里面没有任何文件，只有空的子文件夹。个别程序需要事先存在的空文件夹，删除前请留意。",
    },
    {
      kind: "emptyFile",
      title: "空文件",
      note: "大小为 0 的文件。有些程序用空文件做标记（例如 __init__.py、.gitkeep），删除前请确认用途。",
    },
  ];
  /** 每类最多显示的项数，展开后显示全部。 */
  const PREVIEW = 50;

  let running = $state(false);
  let done = $state(0);
  let total = $state(0);
  let result = $state<FindResult | null>(null);
  let error = $state("");
  const marked = new SvelteSet<number>();
  const expanded = new SvelteSet<FindingKind>();

  const items = $derived((result?.items ?? []).filter((f) => !removed.has(f.id)));
  const byKind = $derived(
    Object.fromEntries(
      SECTIONS.map((s) => [s.kind, items.filter((f) => f.kind === s.kind)]),
    ) as Record<FindingKind, Finding[]>,
  );
  const markedCount = $derived(items.filter((f) => marked.has(f.id)).length);

  async function start() {
    running = true;
    error = "";
    result = null;
    done = 0;
    total = 0;
    marked.clear();
    expanded.clear();
    try {
      result = await findIssues(rootId, (d, t) => {
        done = d;
        total = t;
      });
    } catch (e) {
      if (String(e) !== "已取消") error = String(e);
    } finally {
      running = false;
    }
  }

  function toggle(id: number) {
    if (marked.has(id)) marked.delete(id);
    else marked.add(id);
  }

  function toggleAll(list: Finding[]) {
    const all = list.every((f) => marked.has(f.id));
    for (const f of list) {
      if (all) marked.delete(f.id);
      else marked.add(f.id);
    }
  }

  function split(path: string) {
    const i = path.lastIndexOf("\\");
    return { dir: path.slice(0, i + 1), name: path.slice(i + 1) };
  }
</script>

<section class="findings" aria-label="空文件夹与失效链接">
  <header>
    <div class="title">
      <h2>空文件夹与失效链接</h2>
      <span>在 {rootName} 中查找空文件夹、空文件，以及指向已不存在位置的链接和快捷方式</span>
    </div>
    {#if running}
      <button type="button" class="secondary" onclick={() => cancelIssues()}>停止</button>
    {:else}
      <button type="button" class="primary" onclick={start}
        >{result ? "重新查找" : "开始查找"}</button
      >
    {/if}
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

  {#if running}
    <div class="state" aria-live="polite">
      <p>
        正在逐个核实{total > 0 ? ` · ${done.toLocaleString()} / ${total.toLocaleString()}` : "…"}
      </p>
      <div class="progress" role="progressbar">
        <span style:width="{total > 0 ? (done / total) * 100 : 0}%"></span>
      </div>
    </div>
  {:else if !result}
    <div class="state">
      <p>先从扫描结果里找出可能的项目，再到磁盘上逐个核实，宁可漏报也不误报。</p>
      <p class="muted">系统保护的位置不会列出。</p>
    </div>
  {:else if items.length === 0}
    <div class="state">
      <p>{result.items.length > 0 ? "都已处理。" : "没有找到空文件夹或失效的链接。"}</p>
      <p class="muted">
        核实了 {result.checked.toLocaleString()} 个可能的项目，用时 {(
          result.elapsedMs / 1000
        ).toFixed(1)} 秒。
      </p>
    </div>
  {:else}
    <div class="list">
      {#each SECTIONS as s (s.kind)}
        {@const list = byKind[s.kind]}
        {#if list.length > 0}
          <article class="section">
            <h3>
              <span>{s.title}</span>
              <span class="count">{list.length.toLocaleString()}</span>
              <button type="button" class="link" onclick={() => toggleAll(list)}
                >{list.every((f) => marked.has(f.id)) ? "全部取消" : "全选"}</button
              >
            </h3>
            <p class="note">{s.note}</p>
            <ul>
              {#each expanded.has(s.kind) ? list : list.slice(0, PREVIEW) as f (f.id)}
                {@const p = split(f.path)}
                <li>
                  <input
                    type="checkbox"
                    checked={marked.has(f.id)}
                    onchange={() => toggle(f.id)}
                    aria-label="删除 {f.path}"
                  />
                  <button
                    type="button"
                    class="path"
                    title="在主界面中定位"
                    onclick={() => onreveal(f.id)}
                    ><span class="dir">{p.dir}</span><span>{p.name}</span></button
                  >
                  {#if f.target}
                    <span class="target" title={f.target}>→ {f.target}</span>
                  {/if}
                </li>
              {/each}
            </ul>
            {#if list.length > PREVIEW && !expanded.has(s.kind)}
              <button type="button" class="link more" onclick={() => expanded.add(s.kind)}
                >显示全部 {list.length.toLocaleString()} 项</button
              >
            {/if}
          </article>
        {/if}
      {/each}
      {#if result.more > 0}
        <p class="muted">另有 {result.more.toLocaleString()} 项因数量太多未列出。</p>
      {/if}
    </div>

    <footer>
      <span>已勾选 <strong>{markedCount.toLocaleString()}</strong> 项</span>
      <button type="button" class="link" onclick={() => marked.clear()} disabled={markedCount === 0}
        >全部取消</button
      >
      <button
        type="button"
        class="primary"
        disabled={markedCount === 0}
        onclick={() => ondelete(items.filter((f) => marked.has(f.id)).map((f) => f.id))}
        >删除勾选的项目…</button
      >
    </footer>
  {/if}
</section>

<style>
  .findings {
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
  .muted,
  .note {
    color: var(--ink-2);
    font-size: 12px;
  }

  button.primary,
  button.secondary {
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    font-weight: 500;
  }

  button.primary {
    background: var(--accent);
    color: var(--accent-ink);
  }

  button.secondary {
    background: var(--field);
  }

  button:disabled {
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
    background: var(--danger-bg);
    color: var(--danger);
  }

  .state {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    text-align: center;
  }

  .state p {
    margin: 0;
  }

  .progress {
    width: min(360px, 80%);
    height: 6px;
    margin: 6px 0;
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

  .list {
    flex: 1;
    min-height: 0;
    padding: 8px 12px;
    overflow-y: auto;
  }

  .section {
    margin: 0 0 10px;
    padding-bottom: 6px;
    border-radius: 10px;
    background: var(--panel-2);
  }

  h3 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0;
    padding: 10px 12px 2px;
    font-size: 13px;
  }

  .count {
    padding: 0 7px;
    border-radius: 999px;
    background: var(--field);
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
  }

  h3 .link {
    margin-left: auto;
  }

  .note {
    margin: 0;
    padding: 0 12px 6px;
  }

  ul {
    margin: 0;
    padding: 0 4px;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 3px 8px;
    border-radius: 6px;
    font-size: 12px;
  }

  li:hover {
    background: var(--hover);
  }

  input[type="checkbox"] {
    margin: 0;
    accent-color: var(--danger);
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

  .target {
    max-width: 320px;
    overflow: hidden;
    color: var(--danger);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font-size: 12px;
  }

  .more {
    margin: 4px 12px 0;
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
</style>

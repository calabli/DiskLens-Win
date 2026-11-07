<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { devProjects, type ProjectInfo } from "$lib/api";
  import { formatAgo, formatSize } from "$lib/format";

  interface Props {
    rootId: number;
    rootName: string;
    /** 数据版本；删除或刷新后递增，重新查找。 */
    version: number;
    ondelete: (ids: number[]) => void;
    onbasket: (ids: number[]) => void;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { rootId, rootName, version, ondelete, onbasket, onreveal, onclose }: Props = $props();

  const DAY = 86_400;
  /** 多久没改算“很久没动”。 */
  const STALE_DAYS = 90;
  const AGES = [
    { days: 0, label: "全部项目" },
    { days: 90, label: "90 天没改过" },
    { days: 180, label: "半年没改过" },
    { days: 365, label: "一年没改过" },
  ];

  let list = $state<ProjectInfo[] | null>(null);
  let error = $state("");
  let age = $state(0);
  let showEmpty = $state(false);
  const marked = new SvelteSet<number>();

  $effect(() => {
    const id = rootId;
    void version;
    list = null;
    devProjects(id)
      .then((l) => {
        list = l;
        error = "";
        const present = new Set(l.filter((p) => p.reclaimable > 0).map((p) => p.id));
        for (const m of [...marked]) if (!present.has(m)) marked.delete(m);
      })
      .catch((e) => (error = String(e)));
  });

  const now = Math.floor(Date.now() / 1000);
  const idle = (p: ProjectInfo) => (p.modified > 0 ? Math.floor((now - p.modified) / DAY) : -1);

  const shown = $derived(
    (list ?? []).filter((p) => (showEmpty || p.reclaimable > 0) && (age === 0 || idle(p) >= age)),
  );
  const totalReclaimable = $derived(shown.reduce((s, p) => s + p.reclaimable, 0));
  const markedList = $derived(shown.filter((p) => marked.has(p.id)));
  const markedSize = $derived(markedList.reduce((s, p) => s + p.reclaimable, 0));
  const markedArtifacts = $derived(markedList.flatMap((p) => p.artifacts.map((a) => a.id)));

  function toggle(p: ProjectInfo) {
    if (marked.has(p.id)) marked.delete(p.id);
    else marked.add(p.id);
  }

  function markAll() {
    for (const p of shown) if (p.reclaimable > 0) marked.add(p.id);
  }

  function name(path: string) {
    return path.slice(path.lastIndexOf("\\") + 1) || path;
  }
</script>

<section class="projects" aria-label="开发项目">
  <header>
    <div class="title">
      <h2>开发项目</h2>
      <span
        >在 {rootName} 中找出开发项目，以及可以删掉、之后能重新生成的依赖包和编译输出。删掉后再运行项目时，按提示重新安装或编译就行。</span
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

  <div class="bar">
    <select bind:value={age} aria-label="按最后修改时间筛选">
      {#each AGES as a (a.days)}
        <option value={a.days}>{a.label}</option>
      {/each}
    </select>
    <label class="check">
      <input type="checkbox" bind:checked={showEmpty} />
      也显示没有构建产物的项目
    </label>
    <button type="button" class="link" onclick={markAll} disabled={shown.length === 0}>全选</button>
    <button type="button" class="link" onclick={() => marked.clear()} disabled={marked.size === 0}
      >全部取消</button
    >
  </div>

  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}

  <div class="scroll">
    {#if list === null}
      <p class="muted">正在查找项目…</p>
    {:else if shown.length === 0}
      <p class="muted">
        {list.length === 0 ? `${rootName} 里没有找到开发项目。` : "没有符合条件的项目。"}
      </p>
    {:else}
      <ul>
        {#each shown as p (p.id)}
          {@const days = idle(p)}
          <li class:on={marked.has(p.id)}>
            <input
              type="checkbox"
              checked={marked.has(p.id)}
              disabled={p.reclaimable === 0}
              onchange={() => toggle(p)}
              aria-label="选择 {name(p.path)}"
            />
            <div class="main">
              <div class="line">
                <button
                  type="button"
                  class="name"
                  title="在主界面中定位"
                  onclick={() => onreveal(p.id)}>{name(p.path)}</button
                >
                {#each p.kinds as k (k)}
                  <span class="kind">{k}</span>
                {/each}
                {#if days >= STALE_DAYS}
                  <span class="stale">{days} 天没改过</span>
                {/if}
              </div>
              <span class="path" title={p.path}>{p.path}</span>
              <div class="artifacts">
                {#each p.artifacts as a (a.id)}
                  <button
                    type="button"
                    class="artifact"
                    title="在主界面中定位"
                    onclick={() => onreveal(a.id)}>{a.name} <b>{formatSize(a.size)}</b></button
                  >
                {:else}
                  <span class="muted">没有构建产物</span>
                {/each}
              </div>
            </div>
            <div class="side">
              <strong>{formatSize(p.reclaimable)}</strong>
              <span class="muted"
                >{p.modified > 0 ? `${formatAgo(p.modified * 1000)}改过` : "修改时间未知"}</span
              >
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <footer>
    <span
      >共 {shown.length.toLocaleString()} 个项目，构建产物 {formatSize(totalReclaimable)}；已选
      <strong>{markedList.length}</strong> 个，可释放
      <strong>{formatSize(markedSize)}</strong></span
    >
    <button
      type="button"
      class="secondary"
      disabled={markedArtifacts.length === 0}
      onclick={() => onbasket(markedArtifacts)}>加入收集篮</button
    >
    <button
      type="button"
      class="primary"
      disabled={markedArtifacts.length === 0}
      onclick={() => ondelete(markedArtifacts)}>删除所选项目的构建产物…</button
    >
  </footer>
</section>

<style>
  .projects {
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

  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
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

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
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
    padding: 8px 16px 12px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: start;
    margin-bottom: 6px;
    padding: 10px 12px;
    border: 1.5px solid transparent;
    border-radius: 10px;
    background: var(--panel-2);
  }

  li.on {
    border-color: var(--accent-soft);
  }

  li input {
    margin-top: 3px;
    accent-color: var(--accent);
  }

  .main {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .line {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }

  .name {
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .name:hover {
    color: var(--accent);
  }

  .kind {
    padding: 0 7px;
    border-radius: 999px;
    background: var(--field);
    color: var(--ink-2);
    font-size: 11px;
  }

  .stale {
    padding: 0 7px;
    border-radius: 999px;
    background: var(--warn-bg);
    color: var(--warn);
    font-size: 11px;
    font-weight: 600;
  }

  .path {
    overflow: hidden;
    color: var(--ink-2);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .artifacts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }

  .artifact {
    padding: 1px 8px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .artifact b {
    margin-left: 4px;
    color: var(--ink-2);
    font-weight: 500;
  }

  .artifact:hover {
    background: var(--accent-soft);
  }

  .side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .link {
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--accent);
  }

  .link:disabled {
    opacity: 0.4;
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
    font-weight: 500;
  }

  .primary {
    background: var(--accent);
    color: var(--accent-ink);
  }

  .secondary {
    background: var(--field);
    color: var(--ink);
  }

  .primary:disabled,
  .secondary:disabled {
    opacity: 0.45;
  }
</style>

<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import {
    cancelDuplicates,
    findDuplicates,
    type DupFile,
    type DupGroup,
    type DupProgress,
    type DupResult,
    type LinkGroup,
  } from "$lib/api";
  import { formatSize } from "$lib/format";

  interface Props {
    /** 在哪个目录下查找。 */
    rootId: number;
    rootName: string;
    /** 已经删除的项目，从结果里去掉。 */
    removed: Set<number>;
    /** 把勾选的文件交给外层确认并删除。 */
    ondelete: (ids: number[]) => void;
    /** 把勾选的文件替换为指向保留那份的硬链接（外层执行并提示结果）。 */
    onlink: (groups: LinkGroup[]) => Promise<void>;
    /** 在主界面中定位这个文件。 */
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { rootId, rootName, removed, ondelete, onlink, onreveal, onclose }: Props = $props();

  type Keep = "shortest" | "newest" | "oldest";

  const SIZES = [
    { label: "全部", value: 1 },
    { label: "≥ 1 MB", value: 1 << 20 },
    { label: "≥ 10 MB", value: 10 << 20 },
    { label: "≥ 100 MB", value: 100 << 20 },
    { label: "≥ 1 GB", value: 1 << 30 },
  ];
  const KEEP: { key: Keep; label: string }[] = [
    { key: "shortest", label: "路径最短" },
    { key: "newest", label: "最新" },
    { key: "oldest", label: "最旧" },
  ];
  /** 一次显示的组数，滚动到底再多显示。 */
  const PAGE = 100;

  let minSize = $state(1 << 20);
  let keep = $state<Keep>("shortest");
  let running = $state(false);
  let progress = $state<DupProgress | null>(null);
  let result = $state<DupResult | null>(null);
  let error = $state("");
  let shown = $state(PAGE);
  /** 勾选（要删除）的文件。 */
  const marked = new SvelteSet<number>();

  const groups = $derived.by(() => {
    if (!result) return [];
    return result.groups
      .map((g) => ({ ...g, files: g.files.filter((f) => !removed.has(f.id)) }))
      .filter((g) => g.files.length > 1);
  });
  const wasted = $derived(groups.reduce((sum, g) => sum + g.size * (g.files.length - 1), 0));
  /** 只算真正能释放的：还有别的硬链接名称的文件删掉也不释放空间。 */
  const markedSize = $derived(
    groups.reduce(
      (sum, g) => sum + g.size * g.files.filter((f) => marked.has(f.id) && f.links <= 1).length,
      0,
    ),
  );
  const markedCount = $derived(
    groups.reduce((sum, g) => sum + g.files.filter((f) => marked.has(f.id)).length, 0),
  );

  /** 按规则选出组里要保留的那一份。有其他硬链接名称的优先保留，因为删掉它不释放空间。 */
  function keeper(g: DupGroup): DupFile {
    const by = {
      shortest: (a: DupFile, b: DupFile) => a.path.length - b.path.length,
      newest: (a: DupFile, b: DupFile) => b.modifiedMs - a.modifiedMs,
      oldest: (a: DupFile, b: DupFile) => a.modifiedMs - b.modifiedMs,
    }[keep];
    const linked = (f: DupFile) => (f.links > 1 ? 0 : 1);
    return [...g.files].sort(
      (a, b) => linked(a) - linked(b) || by(a, b) || a.path.localeCompare(b.path),
    )[0];
  }

  /** 每组保留一份，其余全部勾选。 */
  function applyKeep() {
    marked.clear();
    for (const g of groups) {
      const k = keeper(g);
      for (const f of g.files) if (f.id !== k.id) marked.add(f.id);
    }
  }

  async function start() {
    running = true;
    error = "";
    result = null;
    progress = null;
    marked.clear();
    shown = PAGE;
    try {
      result = await findDuplicates(rootId, minSize, (p) => (progress = p));
      applyKeep();
    } catch (e) {
      if (String(e) !== "已取消") error = String(e);
    } finally {
      running = false;
    }
  }

  /**
   * 硬链接替换的计划：每组以保留的那份为准，替换勾选的、与它在同一个卷上、且自身没有
   * 其他硬链接名称的文件（有其他名称的替换了也不释放空间）。
   */
  const linkPlan = $derived.by(() => {
    const plan: LinkGroup[] = [];
    let size = 0;
    let count = 0;
    let crossVolume = 0;
    let linked = 0;
    for (const g of groups) {
      const kept = g.files.filter((f) => !marked.has(f.id));
      const keep = kept.find((f) => f.links > 1) ?? kept[0];
      if (!keep) continue;
      const replace: number[] = [];
      for (const f of g.files) {
        if (!marked.has(f.id)) continue;
        if (f.volume !== keep.volume) crossVolume++;
        else if (f.links > 1) linked++;
        else {
          replace.push(f.id);
          size += g.size;
        }
      }
      if (replace.length > 0) {
        plan.push({ keep: keep.id, replace });
        count += replace.length;
      }
    }
    return { groups: plan, size, count, crossVolume, linked };
  });

  let confirmLink = $state(false);
  let linking = $state(false);

  async function link() {
    linking = true;
    try {
      await onlink(linkPlan.groups);
    } finally {
      linking = false;
      confirmLink = false;
    }
  }

  function toggle(g: DupGroup, f: DupFile) {
    if (marked.has(f.id)) {
      marked.delete(f.id);
      return;
    }
    // 每组至少保留一份。
    const others = g.files.filter((x) => x.id !== f.id);
    if (others.every((x) => marked.has(x.id))) return;
    marked.add(f.id);
  }

  function split(path: string) {
    const i = path.lastIndexOf("\\");
    return { dir: path.slice(0, i + 1), name: path.slice(i + 1) };
  }

  function date(ms: number) {
    return new Date(ms).toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function onscroll(event: Event) {
    const el = event.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400 && shown < groups.length) {
      shown += PAGE;
    }
  }

  const percent = $derived(
    progress && progress.bytesTotal > 0 ? (progress.bytesDone / progress.bytesTotal) * 100 : 0,
  );
</script>

<section class="dupes" aria-label="重复文件">
  <header>
    <div class="title">
      <h2>重复文件</h2>
      <span>在 {rootName} 中查找内容完全相同的文件</span>
    </div>
    <label class="size">
      大小
      <select bind:value={minSize} disabled={running}>
        {#each SIZES as s (s.value)}
          <option value={s.value}>{s.label}</option>
        {/each}
      </select>
    </label>
    {#if running}
      <button type="button" class="secondary" onclick={() => cancelDuplicates()}>停止</button>
    {:else}
      <button type="button" class="primary" onclick={start}
        >{result ? "重新查找" : "开始查找"}</button
      >
    {/if}
    <button type="button" class="close" onclick={onclose} aria-label="关闭重复文件">
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
        {progress?.phase === "hashing" ? "正在比对文件内容" : "正在核对大小相同的文件"}
        {#if progress}
          · {progress.filesDone.toLocaleString()} / {progress.filesTotal.toLocaleString()}
        {/if}
      </p>
      <div class="progress" role="progressbar" aria-valuenow={Math.round(percent)}>
        <span style:width="{percent}%"></span>
      </div>
      {#if progress && progress.phase === "hashing"}
        <p class="muted">{formatSize(progress.bytesDone)} / {formatSize(progress.bytesTotal)}</p>
      {/if}
    </div>
  {:else if !result}
    <div class="state">
      <p>先按大小筛选，再比较文件头尾，最后用完整内容的哈希逐一核对。</p>
      <p class="muted">算过的结果会记下来，文件没变就不必再读。</p>
    </div>
  {:else if groups.length === 0}
    <div class="state">
      <p>{result.groups.length > 0 ? "重复文件都已处理。" : "没有找到重复文件。"}</p>
      <p class="muted">
        核对了 {result.candidates.toLocaleString()} 个大小相同的文件，用时 {(
          result.elapsedMs / 1000
        ).toFixed(1)} 秒。
      </p>
    </div>
  {:else}
    <div class="summary">
      <span
        ><strong>{groups.length.toLocaleString()} 组</strong>，全部只留一份可释放
        <strong>{formatSize(wasted)}</strong></span
      >
      <span class="muted"
        >用时 {(result.elapsedMs / 1000).toFixed(1)} 秒{result.skipped > 0
          ? `，${result.skipped} 个文件读不了已跳过`
          : ""}{result.moreGroups > 0 ? `，另有 ${result.moreGroups} 组较小的未列出` : ""}</span
      >
      <div class="keep" role="radiogroup" aria-label="每组保留">
        <span>每组保留</span>
        {#each KEEP as k (k.key)}
          <button
            type="button"
            role="radio"
            aria-checked={keep === k.key}
            onclick={() => {
              keep = k.key;
              applyKeep();
            }}>{k.label}</button
          >
        {/each}
      </div>
    </div>

    <div class="groups" {onscroll}>
      {#each groups.slice(0, shown) as g (g.files[0].id)}
        <article class="group">
          <h3>
            <span class="name">{split(g.files[0].path).name}</span>
            <span class="muted"
              >{formatSize(g.size)} × {g.files.length}，可释放 {formatSize(
                g.size * (g.files.length - 1),
              )}</span
            >
          </h3>
          <ul>
            {#each g.files as f (f.id)}
              {@const p = split(f.path)}
              <li class:kept={!marked.has(f.id)}>
                <input
                  type="checkbox"
                  checked={marked.has(f.id)}
                  onchange={(e) => {
                    toggle(g, f);
                    (e.currentTarget as HTMLInputElement).checked = marked.has(f.id);
                  }}
                  aria-label="删除 {f.path}"
                />
                <button
                  type="button"
                  class="path"
                  title="在主界面中定位"
                  onclick={() => onreveal(f.id)}
                  ><span class="dir">{p.dir}</span><span>{p.name}</span></button
                >
                <span class="date"
                  >{#if f.links > 1}<span
                      class="link-note"
                      title="这个文件还有 {f.links -
                        1} 个其他名称（硬链接），删除这个名称不会释放空间">硬链接</span
                    >{/if}{date(f.modifiedMs)}</span
                >
                <span class="tag">{marked.has(f.id) ? "" : "保留"}</span>
              </li>
            {/each}
          </ul>
        </article>
      {/each}
    </div>

    <footer>
      <span
        >已勾选 <strong>{markedCount.toLocaleString()}</strong> 个文件，可释放
        <strong>{formatSize(markedSize)}</strong></span
      >
      <button type="button" class="link" onclick={() => marked.clear()} disabled={markedCount === 0}
        >全部取消</button
      >
      <button
        type="button"
        class="secondary"
        disabled={linkPlan.count === 0}
        title="保留一份数据，其余位置换成指向它的硬链接，文件仍在原处"
        onclick={() => (confirmLink = true)}>替换为硬链接…</button
      >
      <button
        type="button"
        class="primary"
        disabled={markedCount === 0}
        onclick={() =>
          ondelete(groups.flatMap((g) => g.files.filter((f) => marked.has(f.id)).map((f) => f.id)))}
        >删除勾选的文件…</button
      >
    </footer>
  {/if}
</section>

{#if confirmLink}
  <div class="backdrop" transition:fade={{ duration: 150 }} role="presentation"></div>
  <div
    class="sheet"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="link-title"
    transition:fly={{ y: -24, duration: 220, easing: cubicOut }}
  >
    <h2 id="link-title">要将 {linkPlan.count} 个文件替换为硬链接吗？</h2>
    <p>
      可释放 <strong>{formatSize(linkPlan.size)}</strong
      >。这些文件仍留在原来的位置、照常打开，但会和保留的那份共用同一份数据：
      <strong>修改其中任何一处，其余位置也会跟着变</strong>。
    </p>
    <p class="muted">
      替换前会逐字节核对内容，不一致的不替换。被替换的副本不进回收站，因为同样的内容仍在保留的那份里。
    </p>
    {#if linkPlan.crossVolume > 0}
      <p class="muted">
        另有 {linkPlan.crossVolume} 个文件与保留的那份不在同一个磁盘上，不能做硬链接，将跳过。
      </p>
    {/if}
    {#if linkPlan.linked > 0}
      <p class="muted">
        另有 {linkPlan.linked} 个文件本身已有其他硬链接名称，替换也不释放空间，将跳过。
      </p>
    {/if}
    <div class="actions">
      <button
        type="button"
        class="secondary"
        onclick={() => (confirmLink = false)}
        disabled={linking}>取消</button
      >
      <button type="button" class="primary" onclick={link} disabled={linking}
        >{linking ? "正在替换…" : "替换为硬链接"}</button
      >
    </div>
  </div>
{/if}

<style>
  .dupes {
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

  .size {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ink-2);
    font-size: 12px;
  }

  select {
    height: 26px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    background: var(--field);
    color: var(--ink);
    font: inherit;
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

  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }

  .keep {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
    padding: 2px;
    border-radius: 7px;
    background: var(--field);
    font-size: 12px;
  }

  .keep span {
    padding: 0 6px 0 4px;
    color: var(--ink-2);
  }

  .keep button {
    height: 22px;
    padding: 0 10px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-2);
  }

  .keep button[aria-checked="true"] {
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }

  .groups {
    flex: 1;
    min-height: 0;
    padding: 8px 12px;
    overflow-y: auto;
  }

  .group {
    margin: 0 0 10px;
    border-radius: 10px;
    background: var(--panel-2);
    overflow: hidden;
  }

  h3 {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0;
    padding: 8px 12px 4px;
    font-size: 13px;
  }

  h3 .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  h3 .muted {
    flex: none;
    font-weight: 400;
  }

  ul {
    margin: 0;
    padding: 0 4px 6px;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto 36px;
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

  .path span:last-child {
    flex: none;
    max-width: 70%;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .path .dir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--ink-2);
  }

  .date {
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
  }

  .link-note {
    margin-right: 8px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--warn-bg);
    color: var(--warn);
    font-size: 10px;
  }

  .tag {
    color: var(--good);
    font-size: 11px;
    font-weight: 600;
    text-align: right;
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
    width: min(440px, calc(100vw - 32px));
    padding: 22px 22px 18px;
    border-radius: 14px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    transform: translateX(-50%);
  }

  .sheet h2 {
    margin: 0;
    font-size: 15px;
    text-align: center;
  }

  .sheet p {
    margin: 0;
    line-height: 1.6;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }

  .actions button {
    flex: 1;
    height: 30px;
  }

  .link {
    border: 0;
    background: none;
    color: var(--accent);
  }
</style>

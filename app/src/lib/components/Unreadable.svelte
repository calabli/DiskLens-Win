<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { isElevated, locate, restartAsAdmin, type UnreadableDir } from "$lib/api";

  interface Props {
    /** 扫描的根路径，以管理员身份重新启动后重新扫描它。 */
    root: string;
    /** 读不了的文件夹总数（列表最多只记前一部分）。 */
    total: number;
    dirs: UnreadableDir[];
    /** 在主界面中定位到这一项。 */
    onreveal: (id: number) => void;
    /** 在文件管理器中打开这一项。 */
    onexplore: (id: number) => void;
    onclose: () => void;
  }

  let { root, total, dirs, onreveal, onexplore, onclose }: Props = $props();

  let elevated = $state<boolean | null>(null);
  let error = $state("");
  let copied = $state(false);

  $effect(() => {
    isElevated()
      .then((e) => (elevated = e))
      .catch(() => (elevated = false));
  });

  /** 按原因分组计数，显示在顶部。 */
  const reasons = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const d of dirs) counts.set(d.reason, (counts.get(d.reason) ?? 0) + 1);
    return [...counts].sort((a, b) => b[1] - a[1]);
  });

  async function find(path: string, then: (id: number) => void) {
    error = "";
    try {
      const id = await locate(path);
      if (id == null) error = `扫描结果里找不到 ${path}，可能已被删除`;
      else then(id);
    } catch (e) {
      error = String(e);
    }
  }

  async function restart() {
    error = "";
    try {
      await restartAsAdmin(root);
    } catch (e) {
      // 在 UAC 提示里取消时不必提示。
      if (String(e) !== "已取消") error = String(e);
    }
  }

  async function copyAll() {
    try {
      await navigator.clipboard.writeText(dirs.map((d) => d.path).join("\r\n"));
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch (e) {
      error = String(e);
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }
</script>

<svelte:window {onkeydown} />

<div
  class="backdrop"
  transition:fade={{ duration: 150 }}
  role="presentation"
  onclick={onclose}
></div>
<div
  class="sheet"
  role="dialog"
  aria-modal="true"
  aria-labelledby="unreadable-title"
  transition:fly={{ y: -24, duration: 220, easing: cubicOut }}
>
  <h2 id="unreadable-title">读不到的文件夹（{total.toLocaleString()} 个）</h2>
  <p>
    扫描时打不开这些文件夹，它们里面的文件<strong>没有算进大小</strong>，实际占用会比显示的多。
  </p>
  {#if elevated === false}
    <p class="muted">
      最常见的原因是权限不够。以管理员身份运行 DiskLens
      能读取大多数系统和其他用户的文件夹；扫描整个 NTFS 磁盘时还会改用 MFT
      直读，不受文件夹权限限制，也更快。
    </p>
  {:else if elevated}
    <p class="muted">
      DiskLens 已经以管理员身份运行，这些文件夹仍然打不开，通常是系统专门保护的文件夹，可以忽略。
    </p>
  {/if}

  {#if reasons.length > 1}
    <p class="muted">
      {reasons.map(([r, n]) => `${r} ${n} 个`).join("，")}
    </p>
  {/if}

  <ul class="list">
    {#each dirs as d (d.path)}
      <li>
        <div class="text">
          <span class="path" title={d.path}>{d.path}</span>
          <span class="reason">{d.reason}</span>
        </div>
        <button type="button" class="link" onclick={() => find(d.path, onreveal)}>定位</button>
        <button
          type="button"
          class="link"
          title="在文件管理器中打开；资源管理器可能会询问是否授予访问权限"
          onclick={() => find(d.path, onexplore)}>打开</button
        >
      </li>
    {/each}
  </ul>
  {#if total > dirs.length}
    <p class="muted">只列出了前 {dirs.length.toLocaleString()} 个。</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <button type="button" class="secondary" onclick={copyAll}
      >{copied ? "已复制" : "复制全部路径"}</button
    >
    {#if elevated === false}
      <button type="button" class="primary" onclick={restart}>以管理员身份重新启动并扫描</button>
    {:else}
      <button type="button" class="primary" onclick={onclose}>好</button>
    {/if}
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
    width: min(620px, calc(100vw - 32px));
    max-height: calc(100vh - 128px);
    padding: 22px 22px 18px;
    box-sizing: border-box;
    border-radius: 14px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    transform: translateX(-50%);
  }

  h2 {
    margin: 0;
    font-size: 15px;
    text-align: center;
  }

  p {
    margin: 0;
    line-height: 1.6;
  }

  .muted {
    color: var(--ink-2);
    font-size: 12px;
  }

  .list {
    flex: 1;
    min-height: 80px;
    margin: 0;
    padding: 4px;
    overflow-y: auto;
    list-style: none;
    border-radius: 10px;
    background: var(--field);
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 6px;
  }

  li:hover {
    background: var(--hover);
  }

  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
  }

  .reason {
    color: var(--ink-2);
    font-size: 11px;
  }

  .link {
    flex: none;
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--accent);
  }

  .link:hover {
    background: var(--accent-soft);
  }

  .error {
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
    color: var(--ink);
  }

  .primary {
    background: var(--accent);
    color: var(--accent-ink);
  }
</style>

<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { cleanupHistory, restoreItems, type HistoryGroup, type HistoryItem } from "$lib/api";
  import { formatAgo, formatSize } from "$lib/format";
  import { locale, t, tk } from "$lib/i18n.svelte";

  interface Props {
    /** 数据版本；删除、清理或刷新后递增，重新读取记录。 */
    version: number;
    /** 有项目放回原处之后调用，外层据此刷新扫描结果。 */
    onrestored: () => void;
    onclose: () => void;
  }

  let { version, onrestored, onclose }: Props = $props();

  /** 每批先列出这么多项，其余的点“展开”再看。 */
  const FIRST = 12;

  const ACTION: Record<string, string> = {
    recycle: tk("移到回收站"),
    delete: tk("彻底删除"),
    hardlink: tk("替换为硬链接"),
    move: tk("移到其他磁盘"),
    restore: tk("放回原处"),
  };

  const STATUS: Record<string, { label: string; tone: string }> = {
    inBin: { label: tk("在回收站"), tone: "accent" },
    occupied: { label: tk("原处已有同名项目"), tone: "muted" },
    gone: { label: tk("回收站已清空"), tone: "muted" },
    deleted: { label: tk("已彻底删除"), tone: "muted" },
    notDone: { label: tk("没有删除"), tone: "warn" },
    linked: { label: tk("已替换为硬链接"), tone: "muted" },
    moved: { label: tk("已移走"), tone: "muted" },
    restored: { label: tk("已放回"), tone: "good" },
  };

  let groups = $state<HistoryGroup[] | null>(null);
  let error = $state("");
  let busy = $state(false);
  let notice = $state<{ message: string; detail: string[]; tone: "good" | "warn" } | null>(null);
  const opened = new SvelteSet<number>();

  function load() {
    cleanupHistory()
      .then((g) => {
        groups = g;
        error = "";
      })
      .catch((e) => (error = String(e)));
  }

  $effect(() => {
    void version;
    load();
  });

  const inBinCount = $derived(
    (groups ?? []).reduce((n, g) => n + g.items.filter((i) => i.status === "inBin").length, 0),
  );

  function name(path: string) {
    return path.slice(path.lastIndexOf("\\") + 1) || path;
  }

  function folder(path: string) {
    return path.slice(0, path.lastIndexOf("\\") + 1);
  }

  async function restore(items: HistoryItem[]) {
    if (busy || items.length === 0) return;
    busy = true;
    notice = null;
    try {
      const r = await restoreItems(items.map((i) => ({ time: i.time, path: i.path })));
      const failed = r.failed.slice(0, 5).map((f) => `${f.path}：${f.reason}`);
      if (r.failed.length > 5) failed.push(t("另有 {n} 项没能放回", { n: r.failed.length - 5 }));
      notice = {
        message:
          r.restored.length > 0
            ? t("已放回原处 {n} 项", { n: r.restored.length }) +
              (r.failed.length > 0 ? t("，{n} 项没能放回", { n: r.failed.length }) : "")
            : t("没有放回任何项目"),
        detail: [...failed, ...(r.logError ? [r.logError] : [])],
        tone: r.failed.length > 0 || r.logError ? "warn" : "good",
      };
      if (r.restored.length > 0) onrestored();
      load();
    } catch (e) {
      notice = { message: String(e), detail: [], tone: "warn" };
    } finally {
      busy = false;
    }
  }
</script>

<section class="history" aria-label={t("清理记录")}>
  <header>
    <div class="title">
      <h2>{t("清理记录")}</h2>
      <span
        >{t(
          "每次删除、硬链接替换和移动都记在这里。移到回收站的项目，回收站清空之前都可以放回原处。",
        )}</span
      >
    </div>
    <button type="button" class="close" onclick={onclose} aria-label={t("关闭")}>
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
    <p class="banner warn" role="alert">{error}</p>
  {/if}
  {#if notice}
    <div class="banner {notice.tone}" role="status">
      <p>{notice.message}</p>
      {#each notice.detail as d (d)}
        <p class="detail">{d}</p>
      {/each}
    </div>
  {/if}

  <div class="scroll">
    {#if groups === null}
      <p class="empty">{t("正在读取记录…")}</p>
    {:else if groups.length === 0}
      <p class="empty">
        {t("还没有清理记录。删除、清理、硬链接替换或移动文件夹之后，会在这里列出来。")}
      </p>
    {:else}
      {#if inBinCount > 0}
        <p class="summary">
          {t("共有 {n} 项还在回收站里，可以放回原处。", { n: inBinCount.toLocaleString() })}
        </p>
      {/if}
      {#each groups as g, gi (gi)}
        {@const ok = g.items.filter((i) => i.status !== "notDone")}
        {@const notDone = g.items.length - ok.length}
        {@const back = g.items.filter((i) => i.status === "inBin")}
        <article class="group">
          <div class="group-head">
            <div class="group-title">
              <h3>{ACTION[g.action] ? t(ACTION[g.action]) : g.action}</h3>
              <span title={new Date(g.time).toLocaleString(locale())}
                >{formatAgo(new Date(g.time).getTime())} · {t("{n} 项", {
                  n: ok.length.toLocaleString(),
                })}{g.size > 0 ? ` · ${formatSize(g.size)}` : ""}{notDone > 0
                  ? ` · ${t("{n} 项没有删除", { n: notDone })}`
                  : ""}</span
              >
            </div>
            {#if back.length > 1}
              <button type="button" class="secondary" disabled={busy} onclick={() => restore(back)}
                >{t("全部放回（{n}）", { n: back.length.toLocaleString() })}</button
              >
            {/if}
          </div>
          <ul>
            {#each opened.has(gi) ? g.items : g.items.slice(0, FIRST) as item (item.time + item.path)}
              {@const st = STATUS[item.status]}
              <li>
                <div class="text">
                  <span class="name" title={item.path}>{name(item.path)}</span>
                  <span class="path" title={item.path}
                    >{item.linkTo
                      ? `${folder(item.path)} → ${item.linkTo}`
                      : folder(item.path)}{item.reason ? ` · ${t(item.reason)}` : ""}</span
                  >
                </div>
                <span class="size">{item.size > 0 ? formatSize(item.size) : ""}</span>
                <span class="status {st?.tone ?? 'muted'}">{st ? t(st.label) : item.status}</span>
                {#if item.status === "inBin"}
                  <button type="button" class="link" disabled={busy} onclick={() => restore([item])}
                    >{t("放回原处")}</button
                  >
                {:else}
                  <span class="link-space"></span>
                {/if}
              </li>
            {/each}
          </ul>
          {#if g.items.length > FIRST}
            <button
              type="button"
              class="more"
              onclick={() => (opened.has(gi) ? opened.delete(gi) : opened.add(gi))}
              >{opened.has(gi)
                ? t("收起")
                : t("展开其余 {n} 项", { n: (g.items.length - FIRST).toLocaleString() })}</button
            >
          {/if}
        </article>
      {/each}
    {/if}
  </div>
</section>

<style>
  .history {
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

  .title span {
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

  .banner {
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: 10px;
  }

  .banner p {
    margin: 0;
  }

  .banner .detail {
    font-size: 12px;
    opacity: 0.85;
    word-break: break-all;
  }

  .banner.warn {
    background: var(--warn-bg);
    color: var(--warn);
  }

  .banner.good {
    background: var(--good-bg);
    color: var(--good);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 16px 16px;
  }

  .empty,
  .summary {
    margin: 8px 0 12px;
    color: var(--ink-2);
  }

  .group {
    margin-bottom: 10px;
    padding: 10px 14px;
    border-radius: 10px;
    background: var(--panel-2);
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 4px;
  }

  .group-title {
    display: flex;
    flex: 1;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }

  h3 {
    margin: 0;
    font-size: 13px;
    white-space: nowrap;
  }

  .group-title span {
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .secondary {
    flex: none;
    height: 26px;
    padding: 0 12px;
    border: 0;
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 500;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 76px 116px 64px;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
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

  .size {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .status {
    justify-self: start;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }

  .status.accent {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .status.good {
    background: var(--good-bg);
    color: var(--good);
  }

  .status.warn {
    background: var(--warn-bg);
    color: var(--warn);
  }

  .status.muted {
    background: var(--track);
    color: var(--ink-2);
  }

  .link,
  .more {
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--accent);
    white-space: nowrap;
  }

  .link:hover:not(:disabled),
  .more:hover {
    background: var(--accent-soft);
  }

  .link:disabled,
  .secondary:disabled {
    opacity: 0.45;
  }

  .more {
    margin-top: 4px;
  }
</style>

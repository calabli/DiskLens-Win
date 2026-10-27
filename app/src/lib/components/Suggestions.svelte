<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { cleanSuggestions, type Suggestion } from "$lib/api";
  import { formatSize } from "$lib/format";

  interface Props {
    /** 数据版本；清理或刷新后递增，重新计算建议。 */
    version: number;
    /** 把勾选的规则交给外层确认并清理。 */
    onclean: (ruleIds: string[]) => void;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { version, onclean, onreveal, onclose }: Props = $props();

  const RISK = {
    low: { label: "低风险", tone: "good" },
    medium: { label: "中风险", tone: "warn" },
    high: { label: "高风险", tone: "danger" },
  } as const;

  let items = $state<Suggestion[] | null>(null);
  let error = $state("");
  const picked = new SvelteSet<string>();
  const opened = new SvelteSet<string>();
  /** 第一次拿到建议时按规则设好默认勾选；之后保留用户的选择。 */
  let seeded = false;

  $effect(() => {
    void version;
    cleanSuggestions()
      .then((list) => {
        items = list;
        error = "";
        const present = new Set(list.map((s) => s.id));
        for (const id of [...picked]) if (!present.has(id)) picked.delete(id);
        if (!seeded) {
          for (const s of list) if (s.default && s.action !== "guide") picked.add(s.id);
          seeded = true;
        }
      })
      .catch((e) => (error = String(e)));
  });

  const cleanable = $derived((items ?? []).filter((s) => s.action !== "guide"));
  const guides = $derived((items ?? []).filter((s) => s.action === "guide"));
  const pickedSize = $derived(
    cleanable.filter((s) => picked.has(s.id)).reduce((sum, s) => sum + s.size, 0),
  );

  function toggle(id: string) {
    if (picked.has(id)) picked.delete(id);
    else picked.add(id);
  }

  function toggleOpen(id: string) {
    if (opened.has(id)) opened.delete(id);
    else opened.add(id);
  }
</script>

{#snippet card(s: Suggestion)}
  {@const risk = RISK[s.risk]}
  <article class="card" class:on={picked.has(s.id)}>
    <div class="head">
      {#if s.action !== "guide"}
        <input
          type="checkbox"
          checked={picked.has(s.id)}
          onchange={() => toggle(s.id)}
          aria-label="清理 {s.name}"
        />
      {/if}
      <h3>{s.name}</h3>
      <span class="risk {risk.tone}">{risk.label}</span>
      <span class="size">{formatSize(s.size)}</span>
    </div>
    <p class="desc">{s.description}</p>
    <p class="effect"><strong>删除后：</strong>{s.effect}</p>
    {#if s.guide}
      <p class="guide"><strong>处理方法：</strong>{s.guide}</p>
    {/if}
    <button type="button" class="link" onclick={() => toggleOpen(s.id)}
      >{opened.has(s.id) ? "收起位置" : `${s.count.toLocaleString()} 个位置`}</button
    >
    {#if opened.has(s.id)}
      <ul>
        {#each s.locations as l (l.id)}
          <li>
            <button type="button" class="path" title="在主界面中定位" onclick={() => onreveal(l.id)}
              >{l.path}</button
            >
            <span>{formatSize(l.size)}</span>
          </li>
        {/each}
        {#if s.count > s.locations.length}
          <li class="muted">另有 {(s.count - s.locations.length).toLocaleString()} 个位置</li>
        {/if}
      </ul>
    {/if}
  </article>
{/snippet}

<section class="suggest" aria-label="清理建议">
  <header>
    <div class="title">
      <h2>清理建议</h2>
      <span
        >按内置规则在本次扫描结果中找出可以释放的空间。每一项都说明是什么、删除后会怎样，只有低风险项会默认勾选。</span
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

  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}

  {#if items && items.length === 0}
    <div class="state">
      <p>没有找到可以清理的项目。</p>
      <p class="muted">
        建议只针对本次扫描的范围；扫描系统盘（C:）才能看到系统临时文件和更新缓存。
      </p>
    </div>
  {:else if items}
    <div class="list">
      {#if cleanable.length > 0}
        <h4>可以清理</h4>
        {#each cleanable as s (s.id)}{@render card(s)}{/each}
      {/if}
      {#if guides.length > 0}
        <h4>需要通过系统或程序处理</h4>
        <p class="muted section-note">这些项目直接删除可能带来问题，请按处理方法操作。</p>
        {#each guides as s (s.id)}{@render card(s)}{/each}
      {/if}
      <p class="muted section-note">
        建议只针对本次扫描的范围；扫描系统盘（C:）才能看到系统临时文件和更新缓存。
      </p>
    </div>
    <footer>
      <span
        >已选 <strong>{picked.size}</strong> 项，可释放
        <strong>{formatSize(pickedSize)}</strong></span
      >
      <button
        type="button"
        class="primary"
        disabled={picked.size === 0}
        onclick={() => onclean([...picked])}>清理所选…</button
      >
    </footer>
  {/if}
</section>

<style>
  .suggest {
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

  .list {
    flex: 1;
    min-height: 0;
    padding: 4px 14px 12px;
    overflow-y: auto;
  }

  h4 {
    margin: 12px 2px 8px;
    color: var(--ink-2);
    font-size: 12px;
  }

  .section-note {
    margin: -4px 2px 8px;
  }

  .card {
    margin: 0 0 8px;
    padding: 10px 14px;
    border: 1.5px solid transparent;
    border-radius: 10px;
    background: var(--panel-2);
  }

  .card.on {
    border-color: var(--accent-soft);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .head input {
    margin: 0;
  }

  h3 {
    margin: 0;
    font-size: 13px;
  }

  .risk {
    padding: 1px 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
  }

  .risk.good {
    background: var(--good-bg);
    color: var(--good);
  }

  .risk.warn {
    background: var(--warn-bg);
    color: var(--warn);
  }

  .risk.danger {
    background: var(--danger-bg);
    color: var(--danger);
  }

  .size {
    margin-left: auto;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .card p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.6;
  }

  .desc {
    color: var(--ink-2);
  }

  .guide {
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--field);
    user-select: text;
  }

  .link {
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font-size: 12px;
  }

  ul {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 2px 0;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .path {
    min-width: 0;
    padding: 0;
    overflow: hidden;
    border: 0;
    background: none;
    color: var(--ink-2);
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .path:hover {
    color: var(--accent);
    text-decoration: underline;
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

  .primary {
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 500;
  }

  .primary:disabled {
    opacity: 0.45;
  }
</style>

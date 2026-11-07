<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { checkUpdate, openReleasePage, type UpdateInfo } from "$lib/api";

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let info = $state<UpdateInfo | null>(null);
  let error = $state("");
  let checking = $state(true);

  function check() {
    checking = true;
    error = "";
    checkUpdate()
      .then((i) => (info = i))
      .catch((e) => (error = String(e)))
      .finally(() => (checking = false));
  }

  $effect(() => {
    check();
  });

  /** 发布说明里的一段：标题、列表项、代码块或普通段落。 */
  interface Block {
    kind: "heading" | "item" | "code" | "text";
    text: string;
  }

  /**
   * 把发布说明（Markdown）拆成几种段落，去掉 `**`、反引号这些标记，按纯文本显示，
   * 不把发布说明当作 HTML 插进页面。
   */
  function blocks(md: string): Block[] {
    const out: Block[] = [];
    let code: string[] | null = null;
    const plain = (t: string) => t.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1");
    for (const raw of md.replace(/\r/g, "").split("\n")) {
      const line = raw.trimEnd();
      if (line.startsWith("```")) {
        if (code) {
          out.push({ kind: "code", text: code.join("\n") });
          code = null;
        } else {
          code = [];
        }
      } else if (code) {
        code.push(line);
      } else if (/^#{1,6}\s/.test(line)) {
        out.push({ kind: "heading", text: plain(line.replace(/^#+\s*/, "")) });
      } else if (/^[-*]\s/.test(line)) {
        out.push({ kind: "item", text: plain(line.slice(2)) });
      } else if (line.trim()) {
        out.push({ kind: "text", text: plain(line) });
      }
    }
    if (code) out.push({ kind: "code", text: code.join("\n") });
    return out;
  }

  async function open() {
    if (!info) return;
    try {
      await openReleasePage(info.url);
      onclose();
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
  aria-labelledby="update-title"
  transition:fly={{ y: -24, duration: 220, easing: cubicOut }}
>
  {#if checking}
    <h2 id="update-title">正在检查更新…</h2>
    <p class="muted">正在向 GitHub 查询最新发布的版本。</p>
  {:else if error}
    <h2 id="update-title">没能检查更新</h2>
    <p class="error">{error}</p>
    <p class="muted">请确认网络能访问 GitHub；用了代理的话，DiskLens 会使用系统的代理设置。</p>
  {:else if info?.newer}
    <h2 id="update-title">有新版本 {info.latest}</h2>
    <p class="muted">
      你现在用的是 {info.current}{info.publishedAt
        ? `，新版本发布于 ${new Date(info.publishedAt).toLocaleDateString("zh-CN")}`
        : ""}。下载安装后会覆盖旧版本，扫描结果和设置都会保留。
    </p>
    {#if info.notes.trim()}
      <div class="notes">
        {#each blocks(info.notes) as b, i (i)}
          {#if b.kind === "heading"}
            <h3>{b.text}</h3>
          {:else if b.kind === "item"}
            <p class="item">{b.text}</p>
          {:else if b.kind === "code"}
            <pre>{b.text}</pre>
          {:else}
            <p>{b.text}</p>
          {/if}
        {/each}
      </div>
    {/if}
  {:else if info}
    <h2 id="update-title">已经是最新版本</h2>
    <p class="muted">你现在用的 {info.current} 就是最新发布的版本。</p>
  {/if}

  <p class="privacy">只有点“检查更新”时，DiskLens 才会访问 GitHub，不会在后台自动联网或下载。</p>

  <div class="actions">
    {#if info?.newer}
      <button type="button" class="secondary" onclick={onclose}>以后再说</button>
      <button type="button" class="primary" onclick={open}>打开下载页</button>
    {:else if error}
      <button type="button" class="secondary" onclick={onclose}>关闭</button>
      <button type="button" class="primary" onclick={check}>重试</button>
    {:else}
      <button type="button" class="primary" disabled={checking} onclick={onclose}>好</button>
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
    width: min(520px, calc(100vw - 32px));
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

  .muted,
  .privacy {
    color: var(--ink-2);
    font-size: 12px;
  }

  .privacy {
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }

  .error {
    color: var(--danger);
  }

  .notes {
    flex: 1;
    min-height: 60px;
    margin: 0;
    padding: 10px 12px;
    overflow-y: auto;
    border-radius: 10px;
    background: var(--field);
    font-size: 12px;
    word-break: break-word;
    user-select: text;
  }

  .notes h3 {
    margin: 10px 0 4px;
    font-size: 12.5px;
  }

  .notes h3:first-child {
    margin-top: 0;
  }

  .notes p {
    margin: 0 0 4px;
  }

  .notes .item {
    position: relative;
    padding-left: 14px;
  }

  .notes .item::before {
    content: "•";
    position: absolute;
    left: 2px;
    color: var(--ink-2);
  }

  .notes pre {
    margin: 4px 0;
    overflow-x: auto;
    font-size: 11px;
    white-space: pre;
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

  .primary:disabled {
    opacity: 0.45;
  }
</style>

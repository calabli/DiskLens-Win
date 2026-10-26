<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { DeletePlan } from "$lib/api";
  import { formatSize } from "$lib/format";

  interface Props {
    plan: DeletePlan;
    busy: boolean;
    onconfirm: (permanent: boolean) => void;
    oncancel: () => void;
  }

  let { plan, busy, onconfirm, oncancel }: Props = $props();

  let permanent = $state(false);
  let understood = $state(false);
  let primary = $state<HTMLButtonElement>();

  const deletable = $derived(plan.items.filter((i) => !i.blocked));
  const blocked = $derived(plan.items.filter((i) => i.blocked));
  const first = $derived(deletable[0] ?? plan.items[0]);
  const title = $derived(
    deletable.length === 0
      ? `不能删除“${first.name}”`
      : deletable.length === 1
        ? permanent
          ? `要彻底删除“${first.name}”吗？`
          : `要将“${first.name}”移到回收站吗？`
        : permanent
          ? `要彻底删除这 ${deletable.length} 项吗？`
          : `要将这 ${deletable.length} 项移到回收站吗？`,
  );
  const canConfirm = $derived(deletable.length > 0 && !busy && (!permanent || understood));

  $effect(() => {
    primary?.focus();
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      oncancel();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="backdrop" transition:fade={{ duration: 150 }} role="presentation"></div>
<div
  class="sheet"
  role="alertdialog"
  aria-modal="true"
  aria-labelledby="confirm-title"
  aria-describedby="confirm-desc"
  transition:fly={{ y: -24, duration: 220, easing: cubicOut }}
>
  <div class="icon" class:danger={permanent || deletable.length === 0} aria-hidden="true">
    <svg viewBox="0 0 24 24" width="30" height="30">
      <path
        d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m-9 0 1 12a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2l1-12M10 11v6M14 11v6"
        fill="none"
        stroke="currentColor"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
      ></path>
    </svg>
  </div>

  <h2 id="confirm-title">{title}</h2>

  <div id="confirm-desc" class="desc">
    {#if deletable.length === 1}
      <p class="path">{first.path}</p>
    {/if}
    {#if deletable.length > 0}
      <p>
        {formatSize(plan.totalSize)}{plan.totalFiles > 1
          ? `，${plan.totalFiles.toLocaleString()} 个文件`
          : ""}。{permanent ? "彻底删除后无法恢复。" : "之后可以在回收站中恢复。"}
      </p>
    {/if}
    {#each blocked as item (item.id)}
      <p class="blocked"><strong>{item.name}</strong>：{item.blocked}</p>
    {/each}
  </div>

  {#if deletable.length > 0}
    <div class="modes" role="radiogroup" aria-label="删除方式">
      <button
        type="button"
        role="radio"
        aria-checked={!permanent}
        class:on={!permanent}
        onclick={() => (permanent = false)}>移到回收站</button
      >
      <button
        type="button"
        role="radio"
        aria-checked={permanent}
        class:on={permanent}
        onclick={() => (permanent = true)}>彻底删除</button
      >
    </div>
    {#if permanent}
      <label class="ack">
        <input type="checkbox" bind:checked={understood} />
        我了解彻底删除后无法恢复
      </label>
    {/if}
  {/if}

  <p class="log">每次操作都会记录到日志：{plan.logDir}</p>

  <div class="actions">
    {#if deletable.length > 0}
      <button type="button" class="secondary" onclick={oncancel} disabled={busy}>取消</button>
      <button
        type="button"
        class="primary"
        class:danger={permanent}
        bind:this={primary}
        disabled={!canConfirm}
        onclick={() => onconfirm(permanent)}
      >
        {busy ? "正在删除…" : permanent ? "彻底删除" : "移到回收站"}
      </button>
    {:else}
      <button type="button" class="primary" bind:this={primary} onclick={oncancel}>知道了</button>
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
    width: min(420px, calc(100vw - 32px));
    margin-left: max(-210px, calc(-50vw + 16px));
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 22px 22px 18px;
    border-radius: 14px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
    text-align: center;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border-radius: 14px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .icon.danger {
    background: var(--danger-bg);
    color: var(--danger);
  }

  h2 {
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 600;
    word-break: break-all;
  }

  .desc p {
    margin: 4px 0;
    color: var(--ink-2);
  }

  .desc .path {
    color: var(--ink);
    font-size: 12px;
    word-break: break-all;
  }

  .desc .blocked {
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--danger-bg);
    color: var(--danger);
    text-align: left;
  }

  .modes {
    display: flex;
    padding: 2px;
    border-radius: 8px;
    background: var(--field);
  }

  .modes button {
    padding: 4px 14px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-2);
  }

  .modes button.on {
    background: var(--panel);
    color: var(--ink);
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.15);
  }

  .ack {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--danger);
  }

  .ack input {
    accent-color: var(--danger);
  }

  .log {
    margin: 2px 0 0;
    color: var(--ink-3);
    font-size: 11px;
    word-break: break-all;
  }

  .actions {
    display: flex;
    gap: 8px;
    align-self: stretch;
    margin-top: 6px;
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

  .primary.danger {
    background: var(--danger);
  }

  .actions button:disabled {
    opacity: 0.45;
  }
</style>

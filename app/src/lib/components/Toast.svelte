<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  interface Props {
    message: string;
    detail?: string[];
    tone?: "info" | "warn";
    onclose: () => void;
  }

  let { message, detail = [], tone = "info", onclose }: Props = $props();

  // 没有需要细看的内容时，几秒后自动消失。
  $effect(() => {
    if (detail.length > 0) return;
    const timer = setTimeout(onclose, 4000);
    return () => clearTimeout(timer);
  });
</script>

<div
  class="toast"
  class:warn={tone === "warn"}
  role="status"
  transition:fly={{ y: 16, duration: 220, easing: cubicOut }}
>
  <div class="text">
    <strong>{message}</strong>
    {#each detail as line (line)}
      <span>{line}</span>
    {/each}
  </div>
  <button type="button" onclick={onclose} aria-label="关闭">
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"
      ><path d="m4 4 8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"
      ></path></svg
    >
  </button>
</div>

<style>
  .toast {
    position: fixed;
    bottom: 72px;
    left: 50%;
    z-index: 30;
    transform: translateX(-50%);
    display: flex;
    align-items: flex-start;
    gap: 12px;
    max-width: min(560px, calc(100vw - 32px));
    padding: 10px 10px 10px 16px;
    border-radius: 14px;
    background: var(--panel);
    box-shadow: var(--shadow-lift);
  }

  .toast.warn strong {
    color: var(--warn);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .text span {
    color: var(--ink-2);
    font-size: 12px;
    word-break: break-all;
  }

  button {
    display: grid;
    place-items: center;
    flex: none;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 50%;
    background: var(--field);
    color: var(--ink-2);
  }
</style>

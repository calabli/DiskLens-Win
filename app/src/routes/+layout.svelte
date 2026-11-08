<script lang="ts">
  import "../app.css";
  import { initLocale } from "$lib/i18n.svelte";
  import { initTheme } from "$lib/theme";

  initLocale();
  initTheme();

  let { children } = $props();
</script>

{@render children()}

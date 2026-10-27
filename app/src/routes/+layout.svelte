<script lang="ts">
  import "../app.css";
  import { initTheme } from "$lib/theme";

  initTheme();

  let { children } = $props();
</script>

{@render children()}

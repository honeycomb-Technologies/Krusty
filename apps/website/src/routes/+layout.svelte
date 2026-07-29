<script lang="ts">
  import '../app.css';
  import Wordmark from '$lib/components/Wordmark.svelte';
  import cellUrl from '../../../../assets/branding/mitsuro/mitsuro-cell-flat.svg?url';

  let { children } = $props();
</script>

<div class="site-shell">
  <header class="nav">
    <div class="nav-inner">
      <a class="brand" href="/" aria-label="Mitsuro home">
        <img class="brand-cell" src={cellUrl} alt="" aria-hidden="true" />
        <Wordmark width={104} />
      </a>
      <nav class="nav-links" aria-label="Primary navigation">
        <a href="/#features">Features</a>
        <a href="/plugins">Plugins</a>
        <a href="/#install">Install</a>
        <a class="button ghost" href="https://github.com/honeycomb-Technologies/Krusty">GitHub</a>
      </nav>
    </div>
  </header>

  {@render children()}

  <footer class="footer">
    <div class="page footer-inner">
      <div class="brand">
        <img class="brand-cell" src={cellUrl} alt="" aria-hidden="true" />
        <Wordmark width={104} />
      </div>
      <div>Local-first coding intelligence by Honeycomb Technologies.</div>
    </div>
  </footer>
</div>

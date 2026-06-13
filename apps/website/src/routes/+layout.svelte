<script lang="ts">
  import '../app.css';

  let { children } = $props();
</script>

<div class="site-shell">
  <header class="nav">
    <div class="nav-inner">
      <a class="brand" href="/" aria-label="Krusty home">
        <span class="brand-mark" aria-hidden="true">
          <span></span><span></span><span></span><span></span><span></span><span></span><span></span><span></span><span></span>
        </span>
        <span>Krusty</span>
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
        <span class="brand-mark" aria-hidden="true">
          <span></span><span></span><span></span><span></span><span></span><span></span><span></span><span></span><span></span>
        </span>
        <span>Krusty</span>
      </div>
      <div>Local-first coding agent • hot-reloadable extensions • built with Rust and Svelte.</div>
    </div>
  </footer>
</div>

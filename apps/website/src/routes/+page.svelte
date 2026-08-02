<script lang="ts">
  import Wordmark from '$lib/components/Wordmark.svelte';
  import { plugins } from '$lib/data/catalog';

  const features = [
    {
      title: 'Terminal-native agent',
      body: 'Run Mitsuro locally with tools, project memory, LSP context, and explicit approval controls.'
    },
    {
      title: 'Plugin package runtime',
      body: 'Install npm-shaped packages that can ship native, WASM, and JS/TS plugin shells.'
    },
    {
      title: 'Hot reload for makers',
      body: 'Native plugins shadow-copy dylibs so you can rebuild and reload without restarting the app.'
    },
    {
      title: 'Pocket offerings',
      body: 'Built-in Game Boy Color support proves the rich-plugin path without making emulator state fragile.'
    },
    {
      title: 'Local-first by default',
      body: 'Your workspace remains on your machine. Mitsuro brings the Agent to the code, not the reverse.'
    },
    {
      title: 'Open catalog',
      body: 'The package directory is a static index that can be hosted on GitHub or your own infrastructure.'
    }
  ];

  const installCommand = 'cargo install --git https://github.com/honeycomb-Technologies/Mitsuro --package mitsuro';
</script>

<svelte:head>
  <title>Mitsuro — Local-first AI coding assistant</title>
  <meta name="description" content="Mitsuro is a local-first AI coding assistant from Honeycomb Technologies." />
</svelte:head>

<main class="page">
  <section class="hero">
    <div>
      <div class="eyebrow">local runtime online</div>
      <h1>Code with agents that stay <span class="gradient-text">on your machine.</span></h1>
      <p class="lede">
        Mitsuro is a local-first coding assistant with terminal, desktop, web, and mobile surfaces.
        Agent handles focused work while Hive keeps autonomous work alive across threads.
      </p>
      <div class="actions">
        <a class="button primary" href="/plugins">Browse plugins</a>
        <a class="button" href="#install">Install Mitsuro</a>
        <a class="button ghost" href="https://github.com/honeycomb-Technologies/Mitsuro">View source</a>
      </div>
    </div>

    <div class="terminal-card" aria-label="Mitsuro terminal preview">
      <div class="terminal-titlebar">
        <span>mitsuro / local runtime</span>
        <div class="dots" aria-hidden="true"><span></span><span></span><span></span></div>
      </div>
      <div class="terminal-body">
        <Wordmark />
        <p><span class="prompt">$</span> <span class="command">mitsuro</span></p>
        <p class="comment">booting local agent runtime...</p>
        <p><span class="ok">✓</span> plugins: native • wasm • js/ts</p>
        <p><span class="ok">✓</span> package directory: {plugins.length} official entries</p>
        <p><span class="ok">✓</span> gameboy-color pocket offering ready</p>
        <p class="comment">/plugins opens the searchable catalog</p>
      </div>
    </div>
  </section>

  <section id="features" class="section">
    <div class="section-heading">
      <h2>Built like an editor, not a checkout form.</h2>
      <p>
        No payment splash, no forced login, no cloud-first funnel. The website should explain the
        local runtime and expose the extension ecosystem.
      </p>
    </div>

    <div class="grid">
      {#each features as feature}
        <article class="panel-card">
          <h3>{feature.title}</h3>
          <p>{feature.body}</p>
        </article>
      {/each}
    </div>
  </section>

  <section id="install" class="section">
    <div class="section-heading">
      <h2>Install locally. Extend with packages.</h2>
      <p>
        Plugin packages use Mitsuro's existing compatibility catalog. The website publishes the
        directory; Mitsuro installs from npm, local folders, or custom catalog sources.
      </p>
    </div>

    <div class="install-strip">
      <div><span class="prompt">$</span> <code>{installCommand}</code></div>
      <a class="button primary" href="/plugins">Open package directory</a>
    </div>
  </section>
</main>

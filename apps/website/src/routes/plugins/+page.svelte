<script lang="ts">
  import { plugins, type PluginCatalogEntry } from '$lib/data/catalog';

  let query = $state('');
  let runtime = $state<'all' | PluginCatalogEntry['runtime']>('all');

  const runtimes = ['all', 'native', 'wasm', 'js'] as const;

  const filtered = $derived(
    plugins.filter((plugin) => {
      const text = [
        plugin.id,
        plugin.name,
        plugin.publisher,
        plugin.package,
        plugin.description ?? '',
        plugin.runtime,
        ...plugin.tags
      ]
        .join(' ')
        .toLowerCase();
      const matchesQuery = text.includes(query.trim().toLowerCase());
      const matchesRuntime = runtime === 'all' || plugin.runtime === runtime;
      return matchesQuery && matchesRuntime;
    })
  );

  function installCommand(plugin: PluginCatalogEntry) {
    return `/plugins install ${plugin.package}`;
  }
</script>

<svelte:head>
  <title>Mitsuro Plugin Directory</title>
</svelte:head>

<main class="page">
  <section class="section">
    <div class="section-heading">
      <div>
        <div class="eyebrow">package directory</div>
        <h1 class="plugins-title">Extend Mitsuro.</h1>
      </div>
      <p>
        Official and community plugin packages for Mitsuro. Catalog files are static JSON/TOML and
        can be hosted on GitHub or your own registry URL.
      </p>
    </div>

    <div class="search-row">
      <input
        class="search-input"
        bind:value={query}
        placeholder="Search plugins, runtimes, publishers, tags..."
        aria-label="Search plugins"
      />
      <div class="actions compact">
        {#each runtimes as option}
          <button
            class:primary={runtime === option}
            class="button"
            type="button"
            onclick={() => (runtime = option)}
          >
            {option}
          </button>
        {/each}
      </div>
    </div>

    <div class="plugin-grid">
      {#each filtered as plugin (plugin.id)}
        <article class="plugin-card">
          <div>
            <div class="plugin-card-heading">
              <div>
                <h2>{plugin.name}</h2>
                <p>{plugin.publisher} • v{plugin.version}</p>
              </div>
              {#if plugin.official}
                <span class="pill hot">official</span>
              {/if}
            </div>

            <div class="plugin-meta">
              <span class="pill hot">{plugin.runtime}</span>
              {#each plugin.tags as tag}
                <span class="pill">{tag}</span>
              {/each}
            </div>

            <p>{plugin.description}</p>
          </div>

          <div class="plugin-actions">
            <div class="package-line">{installCommand(plugin)}</div>
            <div class="actions compact">
              {#if plugin.homepage}
                <a class="button" href={plugin.homepage}>Docs</a>
              {/if}
              {#if plugin.repository}
                <a class="button ghost" href={plugin.repository}>Source</a>
              {/if}
            </div>
          </div>
        </article>
      {:else}
        <article class="panel-card">
          <h2>No plugins found.</h2>
          <p>Try another search or runtime filter.</p>
        </article>
      {/each}
    </div>
  </section>

  <section class="section submit-section">
    <div class="section-heading">
      <h2>Publish through npm. List through a catalog.</h2>
      <p>
        Mitsuro installs npm-shaped packages and reads a static catalog. To list a plugin, publish a
        package with the compatibility key <code>mitsuro.plugins</code> in package.json and submit a
        catalog entry.
      </p>
    </div>

    <div class="grid">
      <div class="panel-card">
        <h3>1. Package</h3>
        <p>Ship native dylibs, WASM components, or JS/TS entries in a package.json boundary.</p>
      </div>
      <div class="panel-card">
        <h3>2. Declare</h3>
        <p>Point <code>mitsuro.plugins</code> at one or more plugin.toml manifests.</p>
      </div>
      <div class="panel-card">
        <h3>3. Index</h3>
        <p>Add the package to <code>/plugin-catalog.json</code> or any custom catalog source.</p>
      </div>
    </div>
  </section>
</main>

<style>
  .plugins-title {
    max-width: 760px;
  }

  .compact {
    margin-top: 0;
  }

  .plugin-card-heading {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: 12px;
  }

  .plugin-card h2 {
    font-size: 24px;
    letter-spacing: -0.03em;
  }

  .plugin-actions {
    display: grid;
    gap: 12px;
    margin-top: 22px;
  }

  .submit-section code {
    color: var(--pulse);
    font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  }
</style>

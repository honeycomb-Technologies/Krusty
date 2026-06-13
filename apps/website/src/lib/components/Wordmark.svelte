<script lang="ts">
  const lines = [
    '▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌',
    '█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌',
    '▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪',
    '▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.',
    '·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • '
  ];
</script>

<div class="wordmark" aria-label="Krusty">
  {#each lines as line, index}
    <div style={`--row:${index}`}>{line}</div>
  {/each}
</div>

<style>
  .wordmark {
    display: inline-block;
    color: transparent;
    font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: clamp(9px, 1.42vw, 18px);
    font-weight: 800;
    line-height: 1.05;
    letter-spacing: 0.01em;
    text-align: left;
    text-shadow: 0 0 28px rgba(255, 107, 53, 0.16);
    transform: translateY(0);
    animation: float 3.2s ease-in-out infinite;
  }

  .wordmark div {
    background: linear-gradient(90deg, #8b4513, #cd853f, #ff6b35, #ffcc00, #ff6b35, #8b4513);
    background-size: 260% 100%;
    -webkit-background-clip: text;
    background-clip: text;
    animation: sweep 4s linear infinite;
    animation-delay: calc(var(--row) * -220ms);
  }

  @keyframes sweep {
    from {
      background-position: 0% 50%;
    }
    to {
      background-position: 260% 50%;
    }
  }

  @keyframes float {
    0%,
    100% {
      transform: translateY(0);
    }
    50% {
      transform: translateY(-3px);
    }
  }
</style>

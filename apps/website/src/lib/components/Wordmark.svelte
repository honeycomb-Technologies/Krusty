<script lang="ts">
  import wordmarkUrl from '../../../../../assets/branding/mitsuro/mitsuro-wordmark.svg?url';

  let {
    width = 280,
    label = 'Mitsuro'
  }: {
    width?: number;
    label?: string;
  } = $props();
</script>

<img
  class="wordmark"
  src={wordmarkUrl}
  alt={label}
  style={`width: min(${width}px, 100%);`}
/>

<style>
  .wordmark {
    display: block;
    height: auto;
    color: currentColor;
  }
</style>

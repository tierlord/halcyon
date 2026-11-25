<script lang="ts">
  // Placeholder for the bundle area while bundles and placements load for the
  // first time: the shelf as two tile outlines and three bundle lines, in
  // their final shapes. It is replaced in one step by the real content.
</script>

<div class="skeleton" aria-hidden="true">
  <div class="bar head"></div>
  <div class="tiles">
    <div class="tile"></div>
    <div class="tile"></div>
  </div>
  <div class="line"><span class="bar w1"></span></div>
  <div class="line"><span class="bar w2"></span></div>
  <div class="line"><span class="bar w3"></span></div>
</div>

<style>
  .skeleton {
    --ghost: color-mix(in srgb, var(--text) 7%, transparent);
    --ghost-lit: color-mix(in srgb, var(--text) 11%, transparent);
    display: flex;
    flex-direction: column;
  }
  /* A very slow breath of the surface colour; no motion of shapes. */
  .bar,
  .tile {
    background: var(--ghost);
    animation: breathe 3.2s ease-in-out infinite;
  }
  .head {
    width: 96px;
    height: 14px;
    margin: 2px 0 0 10px;
    border-radius: 7px;
  }
  .tiles {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin: 12px 0 18px;
  }
  /* Same footprint as a shelf tile (BundleShelf.svelte). */
  .tile {
    flex: 1 1 210px;
    height: 60px;
    border-radius: var(--card-radius-sm);
  }
  .line {
    display: flex;
    align-items: center;
    height: 40px;
    padding: 0 10px;
  }
  .line .bar {
    height: 12px;
    border-radius: 6px;
  }
  .w1 {
    width: 62%;
  }
  .w2 {
    width: 48%;
  }
  .w3 {
    width: 55%;
  }
  @keyframes breathe {
    0%,
    100% {
      background: var(--ghost);
    }
    50% {
      background: var(--ghost-lit);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar,
    .tile {
      animation: none;
    }
  }
</style>

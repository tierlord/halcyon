<script lang="ts">
  // The handle of a row or line that has no logo tile: two columns of dots at
  // the left edge, visible on hover or focus of the thing it belongs to. The
  // parent must be `position: relative` and carry `data-assignable` (or, for a
  // bundle header, be a `.bundle-line`).
  import { HANDLE_HINT } from './assignDrag.svelte'
  import { BUNDLE_HANDLE_HINT } from './bundleReorder'

  /**
   * The grip of a bundle header: moves the whole bundle (bundleReorder.ts)
   * instead of assigning a mail. The parent is then the `.bundle-line`.
   */
  let { bundle = false }: { bundle?: boolean } = $props()

  // `aria-description` is not yet in Svelte's typings of HTML attributes.
  const hint = $derived(bundle ? BUNDLE_HANDLE_HINT : HANDLE_HINT)
  const attrs = $derived<Record<string, string>>({
    [bundle ? 'data-bundle-handle' : 'data-drag-handle']: '',
    'aria-description': hint,
  })
</script>

<span class="grip" class:bundle title={hint} {...attrs}>
  <svg width="10" height="16" viewBox="0 0 10 16" aria-hidden="true" fill="currentColor">
    <circle cx="2.5" cy="3" r="1.25" /><circle cx="7.5" cy="3" r="1.25" />
    <circle cx="2.5" cy="8" r="1.25" /><circle cx="7.5" cy="8" r="1.25" />
    <circle cx="2.5" cy="13" r="1.25" /><circle cx="7.5" cy="13" r="1.25" />
  </svg>
</span>

<style>
  .grip {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    z-index: 1;
    width: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-soft);
    opacity: 0;
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
    transition: opacity 0.16s ease;
  }
  :global([data-assignable]:hover) .grip,
  :global([data-assignable]:focus-within) .grip {
    opacity: 0.6;
  }
  :global(.bundle-line:hover) .grip.bundle,
  :global(.bundle-line:focus-within) .grip.bundle {
    opacity: 0.6;
  }
  .grip:hover {
    opacity: 1 !important;
    color: var(--text);
  }
  @media (hover: none) {
    .grip {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .grip {
      transition: none;
    }
  }
</style>

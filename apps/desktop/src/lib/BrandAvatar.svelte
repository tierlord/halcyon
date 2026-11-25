<script lang="ts">
  // The round-cornered logo tile of a sender (initials while there is no logo).
  import { HANDLE_HINT } from './assignDrag.svelte'
  import { useBrand } from './brandView.svelte'
  import { initials } from './format'

  let {
    email,
    name,
    size = 52,
    handle = false,
  }: {
    email: string | null
    name: string
    size?: number
    /** The tile is the drag handle of its card, row or tile. */
    handle?: boolean
  } = $props()

  const brand = useBrand()
  const logo = $derived(brand.logo(email))
</script>

<span
  class="avatar"
  class:has-logo={!!logo}
  class:handle
  style:--logo-bg={logo?.bg ?? 'transparent'}
  style:--size={`${size}px`}
  aria-hidden="true"
  data-drag-handle={handle ? '' : undefined}
  title={handle ? HANDLE_HINT : undefined}
>
  {#if logo}
    <img src={logo.dataUrl} alt="" class:opaque={!logo.padded} />
  {:else}
    {initials(name, email)}
  {/if}
</span>

<style>
  .avatar {
    flex: none;
    width: var(--size);
    height: var(--size);
    border-radius: calc(var(--size) * 0.3);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background: color-mix(in srgb, var(--brand-acc, var(--blue-slate)) 24%, var(--card-surface));
    box-shadow:
      var(--tile-shadow),
      0 0 var(--halo-blur) var(--halo-spread) color-mix(in srgb, var(--brand-acc, transparent) var(--halo-mix), transparent);
    color: var(--text);
    font-size: calc(var(--size) * 0.33);
    font-weight: 650;
  }
  /* Raised by shadow only: the thing under the pointer must not move. */
  .avatar.handle {
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
    transition: box-shadow 0.16s ease;
  }
  .avatar.handle:hover {
    box-shadow:
      var(--tile-shadow),
      0 4px 10px -2px color-mix(in srgb, var(--pitch-black) 32%, transparent),
      0 0 0 2px color-mix(in srgb, var(--brand-acc, var(--highlight)) 55%, transparent);
  }
  .avatar.handle img {
    pointer-events: none;
  }
  :global(body[data-assign-drag]) .avatar.handle {
    cursor: grabbing;
  }
  @media (prefers-reduced-motion: reduce) {
    .avatar.handle {
      transition: none;
    }
  }
  .avatar.has-logo {
    background: var(--logo-bg, white);
  }
  .avatar img {
    width: 70%;
    height: 70%;
    object-fit: contain;
  }
  .avatar img.opaque {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>

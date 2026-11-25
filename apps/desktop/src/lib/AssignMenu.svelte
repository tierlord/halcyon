<script lang="ts">
  // The small "Wohin gehört das?" menu of a card, row or tile. Choosing an
  // entry hands over to the preview scene (AssignLayer); the keyboard way to
  // do what dragging does.
  import { DELETE, type AssignChoice } from './assignDrag.svelte'
  import Icon from './Icon.svelte'
  import PopMenu, { type PopItem } from './PopMenu.svelte'
  import { FOR_YOU, bundleTarget, orderedBundles, sameTarget, type Bundle, type Target } from './bundles'

  let {
    current,
    bundles,
    label,
    onchoose,
  }: {
    /** Where the mail is now: that entry is checked, choosing it does nothing. */
    current: Target
    bundles: Bundle[]
    /** What the button is called for assistive technology. */
    label: string
    /** `null` stands for "Neues Bündel …". `button` is where the scene grows from. */
    onchoose: (target: AssignChoice, button: HTMLElement) => void
  } = $props()

  let open = $state(false)
  let button: HTMLButtonElement | undefined = $state()

  const items = $derived.by<PopItem[]>(() => {
    const targets: { name: string; target: Target }[] = [
      { name: 'Für dich', target: FOR_YOU },
      ...orderedBundles(bundles)
        .filter((b) => b.enabled)
        .map((b) => ({ name: b.name, target: bundleTarget(b.key) })),
    ]
    return [
      ...targets.map(({ name, target }) => ({
        label: name,
        checkable: true,
        checked: sameTarget(target, current),
        action: () => {
          if (!button) return
          // Already there: nothing to do, the focus stays on the button.
          if (sameTarget(target, current)) button.focus()
          else onchoose(target, button)
        },
      })),
      { label: 'Neues Bündel …', separated: true, action: () => button && onchoose(null, button) },
      { label: 'Löschen', danger: true, separated: true, action: () => button && onchoose(DELETE, button) },
    ]
  })
</script>

<button
  bind:this={button}
  type="button"
  class="assign-btn"
  class:open
  aria-label={label}
  title="Wohin gehört das?"
  aria-haspopup="menu"
  aria-expanded={open}
  onclick={(e) => {
    e.stopPropagation()
    open = !open
  }}
>
  <Icon name="more" size={16} />
</button>

{#if open && button}
  <PopMenu anchor={button} heading="Wohin gehört das?" {items} onclose={() => (open = false)} />
{/if}

<style>
  .assign-btn {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 99px;
    background: none;
    color: var(--text-soft);
    opacity: 0.45;
    transition: background 0.16s ease;
  }
  /* Quiet at rest, clear on hover or focus of the thing it belongs to. */
  :global([data-assignable]:hover) .assign-btn,
  :global([data-assignable]:focus-within) .assign-btn,
  .assign-btn.open,
  .assign-btn:focus-visible {
    opacity: 1;
  }
  .assign-btn:hover,
  .assign-btn.open {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  @media (hover: none) {
    .assign-btn {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .assign-btn {
      transition: none;
    }
  }
</style>

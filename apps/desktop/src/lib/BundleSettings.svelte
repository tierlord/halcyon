<script lang="ts">
  // The small settings button of a bundle: what belongs to the bundle, not to
  // a single mail (docs/design/relevanz.md, "Wohin eine Mail gehören kann").
  import type { BundleStore } from './bundleStore.svelte'
  import Icon from './Icon.svelte'
  import NamePrompt from './NamePrompt.svelte'
  import PopMenu, { type PopItem } from './PopMenu.svelte'
  import { OTHER_KEY, type Bundle } from './bundles'

  let {
    bundle,
    store,
    onolder,
    prev,
    next,
  }: {
    bundle: Bundle
    store: BundleStore
    /** Offers "Ältere löschen …"; `null` stands for all. The parent asks for confirmation. */
    onolder?: (days: number | null) => void
    /**
     * The bundle shown above and below on screen, `null` at the ends. Left out
     * where the bundle cannot be moved (the parent shows no order).
     */
    prev?: string | null
    next?: string | null
  } = $props()

  let open = $state(false)
  let confirmDelete = $state(false)
  let choosingOlder = $state(false)
  let renaming = $state(false)
  let button: HTMLButtonElement | undefined = $state()

  const isOther = $derived(bundle.key === OTHER_KEY)

  const items = $derived.by<PopItem[]>(() => {
    if (confirmDelete) {
      return [
        { label: 'Bündel löschen', danger: true, action: () => void store.remove(bundle.key) },
        { label: 'Abbrechen', action: () => {} },
      ]
    }
    if (choosingOlder) {
      return [
        { label: 'Älter als 7 Tage', danger: true, action: () => onolder?.(7) },
        { label: 'Älter als 30 Tage', danger: true, action: () => onolder?.(30) },
        { label: 'Alle', danger: true, action: () => onolder?.(null) },
        { label: 'Abbrechen', separated: true, action: () => {} },
      ]
    }
    const list: PopItem[] = []
    if (!isOther && prev !== undefined && next !== undefined) {
      // Same effect as dragging the header; this is the keyboard way.
      list.push({
        label: 'Nach oben',
        disabled: prev === null,
        action: () => prev && void store.reorder(bundle.key, prev, 'before'),
      })
      list.push({
        label: 'Nach unten',
        disabled: next === null,
        action: () => next && void store.reorder(bundle.key, next, 'after'),
      })
    }
    if (!isOther) {
      list.push({
        separated: list.length > 0,
        label: 'Gleich als gelesen markieren',
        checkable: true,
        checked: bundle.mark_read,
        action: () => void store.update(bundle.key, { markRead: !bundle.mark_read }),
      })
      list.push({ label: 'Ausblenden', action: () => void store.update(bundle.key, { enabled: false }) })
    }
    if (onolder) {
      list.push({
        label: 'Ältere löschen …',
        danger: true,
        separated: true,
        action: () => {
          choosingOlder = true
          open = true
        },
      })
    }
    if (!bundle.builtin) {
      list.push({ label: 'Umbenennen …', separated: true, action: () => (renaming = true) })
      list.push({
        label: 'Löschen …',
        danger: true,
        action: () => {
          confirmDelete = true
          open = true
        },
      })
    }
    return list
  })
  const heading = $derived(
    choosingOlder
      ? 'Welche Mails in den Papierkorb?'
      : confirmDelete ? `‚${bundle.name}‘ löschen? Die Mails landen in „Weitere Mails“.` : bundle.name,
  )

  function close() {
    open = false
    // The confirmation step reopens the menu in the same tick; keep its state.
    queueMicrotask(() => {
      if (!open) {
        confirmDelete = false
        choosingOlder = false
      }
    })
  }
</script>

{#if items.length > 0}
  <button
    bind:this={button}
    type="button"
    class="gear"
    class:open
    aria-label={`Einstellungen für ${bundle.name}`}
    title="Einstellungen"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <Icon name="settings" size={15} />
  </button>
{/if}

{#if open && button}
  {#key `${confirmDelete}${choosingOlder}`}
    <PopMenu anchor={button} {heading} {items} onclose={close} />
  {/key}
{/if}

{#if renaming}
  <NamePrompt
    title="Bündel umbenennen"
    initial={bundle.name}
    confirm="Umbenennen"
    anchor={button ?? null}
    error={store.error}
    onsubmit={async (name) => {
      await store.update(bundle.key, { name })
      if (!store.error) renaming = false
    }}
    oncancel={() => (renaming = false)}
  />
{/if}

<style>
  .gear {
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
    opacity: 0.5;
    transition: background 0.16s ease;
  }
  :global(.bundle-line:hover) .gear,
  :global(.bundle-line:focus-within) .gear,
  .gear.open,
  .gear:focus-visible {
    opacity: 1;
  }
  .gear:hover,
  .gear.open {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  @media (hover: none) {
    .gear {
      opacity: 1;
    }
  }
</style>

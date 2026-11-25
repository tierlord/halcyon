<script lang="ts">
  // The bundles of "Heute" in bundle order: the Übersichten shelf, then one
  // quiet line per non-empty bundle, then "Neues Bündel" and the hidden ones.
  import type { AssignRequest } from './AssignLayer.svelte'
  import type { BundleStore } from './bundleStore.svelte'
  import BundleSection from './BundleSection.svelte'
  import BundleSkeleton from './BundleSkeleton.svelte'
  import BundleShelf from './BundleShelf.svelte'
  import { OTHER_KEY, type BundleSectionModel } from './bundles'
  import Icon from './Icon.svelte'
  import NamePrompt from './NamePrompt.svelte'
  import PopMenu, { type PopItem } from './PopMenu.svelte'

  let {
    sections,
    store,
    onopen,
    assign,
    ondelete,
    ready,
  }: {
    sections: BundleSectionModel[]
    store: BundleStore
    onopen: (id: number) => void
    assign: (req: AssignRequest) => void
    ondelete: (ids: number[], label: string) => void
    /** False until the first complete result; a placeholder stands in until then. */
    ready: boolean
  } = $props()

  let creating = $state(false)
  let hiddenOpen = $state(false)
  let newButton: HTMLButtonElement | undefined = $state()
  let hiddenButton: HTMLButtonElement | undefined = $state()

  /** Bundles on screen above and below, without "Weitere Mails" (nothing goes below it). */
  function neighbours(i: number): { prev: string | null; next: string | null } {
    const key = (j: number) => (sections[j] && sections[j].bundle.key !== OTHER_KEY ? sections[j].bundle.key : null)
    return { prev: key(i - 1), next: key(i + 1) }
  }

  const hidden = $derived(store.bundles.filter((b) => !b.enabled))
  const hiddenItems = $derived<PopItem[]>(
    hidden.map((b) => ({ label: `${b.name} einblenden`, action: () => void store.update(b.key, { enabled: true }) })),
  )
</script>

<div class="bundles" aria-busy={!ready} data-bundle-list>
  {#if !ready}
    <BundleSkeleton />
  {:else}
  {#each sections as s, i (s.bundle.key)}
    <!-- The slot is what bundleReorder.ts measures and slides. -->
    <div class="slot" data-bundle-item={s.bundle.key}>
      {#if s.bundle.presentation === 'shelf'}
        <BundleShelf section={s} {store} {onopen} {assign} {...neighbours(i)} />
      {:else}
        <BundleSection section={s} {store} {onopen} {assign} {ondelete} {...neighbours(i)} />
      {/if}
    </div>
  {/each}

  <div class="foot">
    <button bind:this={newButton} type="button" class="quiet" onclick={() => (creating = true)}>
      <Icon name="plus" size={14} /> Neues Bündel
    </button>
    {#if hidden.length > 0}
      <button
        bind:this={hiddenButton}
        type="button"
        class="quiet"
        aria-haspopup="menu"
        aria-expanded={hiddenOpen}
        onclick={() => (hiddenOpen = !hiddenOpen)}
      >
        Ausgeblendet ({hidden.length})
      </button>
    {/if}
  </div>
  {#if store.error && !creating}<p class="error" role="alert">{store.error}</p>{/if}
  {/if}
</div>

{#if creating}
  <NamePrompt
    title="Neues Bündel"
    confirm="Anlegen"
    anchor={newButton ?? null}
    error={store.error}
    onsubmit={async (name) => {
      if (await store.create(name)) creating = false
    }}
    oncancel={() => (creating = false)}
  />
{/if}

{#if hiddenOpen && hiddenButton}
  <PopMenu
    anchor={hiddenButton}
    heading="Ausgeblendete Bündel"
    items={hiddenItems}
    onclose={() => (hiddenOpen = false)}
  />
{/if}

<style>
  /* Contains the bundle's bottom margin, so a slot's size is its full space. */
  .slot {
    display: flow-root;
  }
  :global(.bundles[data-reordering]) .slot {
    transition: transform 0.2s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  /* The original stays in the flow as a faint place holder in the gap. */
  :global([data-reorder-self]) {
    opacity: 0.3;
  }
  :global(body[data-bundle-drag], body[data-bundle-drag] *) {
    cursor: grabbing !important;
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.bundles[data-reordering]) .slot {
      transition: none;
    }
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 8px;
  }
  .quiet {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-soft);
  }
  .quiet:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  .error {
    margin: 6px 12px 0;
    font-size: 13px;
    color: var(--danger);
  }
</style>

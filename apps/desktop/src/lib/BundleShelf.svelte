<script lang="ts">
  // "Übersichten": the one bundle that stays open as a slim shelf. One small
  // tile per sender with its newest statement; older ones sit behind it.
  import { slide } from 'svelte/transition'
  import type { AssignRequest } from './AssignLayer.svelte'
  import { assignable, dropTarget, type AssignChoice } from './assignDrag.svelte'
  import AssignMenu from './AssignMenu.svelte'
  import BrandAvatar from './BrandAvatar.svelte'
  import type { BundleStore } from './bundleStore.svelte'
  import { reorderable } from './bundleReorder'
  import BundleRows from './BundleRows.svelte'
  import BundleSettings from './BundleSettings.svelte'
  import { bundleTarget, type BundleSectionModel, type ShelfTile, type Target } from './bundles'
  import DragGrip from './DragGrip.svelte'
  import { useBrand } from './brandView.svelte'

  let {
    section,
    store,
    onopen,
    assign,
    prev = null,
    next = null,
  }: {
    section: BundleSectionModel
    store: BundleStore
    onopen: (id: number) => void
    assign: (req: AssignRequest) => void
    /** Neighbouring bundles on screen, for "Nach oben" / "Nach unten"; `null` at the ends. */
    prev?: string | null
    next?: string | null
  } = $props()

  const brand = useBrand()
  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  let openKey = $state<string | null>(null)
  const current: Target = $derived(bundleTarget(section.bundle.key))

  function click(t: ShelfTile) {
    if (t.older.length === 0) onopen(t.newest.message.id)
    else openKey = openKey === t.key ? null : t.key
  }

  function choose(t: ShelfTile, target: AssignChoice, button: HTMLElement) {
    assign({
      messageId: t.newest.message.id,
      target,
      current,
      sourceEl: button.closest<HTMLElement>('[data-assignable]'),
      button,
      name: t.name,
      brandEmail: t.brandEmail,
      style: brand.style(t.brandEmail, t.name),
    })
  }
</script>

<section class="shelf" aria-labelledby="shelf-title">
  <div
    class="shelf-head bundle-line"
    use:dropTarget={current}
    use:reorderable={{ onreorder: (anchor, side) => void store.reorder(section.bundle.key, anchor, side) }}
  >
    <DragGrip bundle />
    <h2 id="shelf-title">{section.bundle.name}</h2>
    <BundleSettings bundle={section.bundle} {store} {prev} {next} />
  </div>
  <ul class="tiles">
    {#each section.tiles as t (t.key)}
      {@const style = brand.style(t.brandEmail, t.name)}
      {@const open = openKey === t.key}
      <li
        class="tile brand-scope"
        class:open
        {style}
        data-assignable
        use:assignable={{
          messageId: t.newest.message.id,
          current,
          label: `${t.name}: ${t.label}`,
          brandEmail: t.brandEmail,
          name: t.name,
          style,
        }}
      >
        <button
          type="button"
          class="tile-main"
          aria-expanded={t.older.length > 0 ? open : undefined}
          aria-controls={open ? 'shelf-panel' : undefined}
          onclick={() => click(t)}
        >
          <BrandAvatar email={t.brandEmail} name={t.name} size={40} handle />
          <span class="texts">
            <span class="label">{t.label}</span>
            <span class="since">{t.since}</span>
          </span>
        </button>
        {#if t.older.length > 0}<span class="stackhint" aria-hidden="true"></span>{/if}
        <span class="menu">
          <AssignMenu
            {current}
            bundles={store.bundles}
            label={`Wohin gehört „${t.label}“?`}
            onchoose={(target, b) => choose(t, target, b)}
          />
        </span>
      </li>
      {#if open}
        <!-- Directly below its tile: a full-width item in the same flow, so the
             next tile moves down instead of the list opening at the far end. -->
        <li class="panel-item" transition:slide={{ duration: reduceMotion ? 0 : 160 }}>
          <div id="shelf-panel" class="panel" role="region" aria-label={`Mails von ${t.name}`}>
            <BundleRows
              rows={[t.newest, ...t.older]}
              bundleKey={section.bundle.key}
              bundles={store.bundles}
              {onopen}
              {assign}
            />
          </div>
        </li>
      {/if}
    {/each}
  </ul>
</section>

<style>
  .shelf {
    margin-bottom: 14px;
  }
  .shelf-head {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 6px 0 18px;
    border-radius: 12px;
  }
  .shelf-head h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 650;
  }
  .tiles {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .tile {
    position: relative;
    flex: 1 1 210px;
    max-width: 100%;
    border-radius: var(--card-radius-sm);
    background: linear-gradient(160deg, var(--tint), var(--tint-end) 72%);
    box-shadow: var(--card-shadow), var(--card-highlight);
  }
  .tile.open {
    box-shadow:
      var(--card-shadow),
      var(--card-highlight),
      0 0 0 2px var(--brand-acc);
  }
  .tile-main {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 36px 10px 10px;
    border: 0;
    border-radius: var(--card-radius-sm);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .texts {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .label {
    font-size: 13.5px;
    font-weight: 650;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .since {
    font-size: 12px;
    color: var(--text-soft);
    line-height: 1.3;
  }
  /* A hint that older ones lie behind the tile. */
  .stackhint {
    position: absolute;
    left: 10px;
    right: 10px;
    bottom: -4px;
    height: 6px;
    border-radius: 0 0 10px 10px;
    background: color-mix(in srgb, var(--brand-acc) 28%, var(--card-surface));
    z-index: -1;
  }
  .menu {
    position: absolute;
    top: 6px;
    right: 4px;
  }
  .panel-item {
    flex: 1 0 100%;
    min-width: 0;
  }
  .panel {
    padding: 4px;
    border-radius: var(--card-radius-sm);
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
  }
</style>

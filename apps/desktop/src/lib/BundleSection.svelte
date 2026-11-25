<script lang="ts">
  // A collapsed bundle: one quiet line with its name and a sentence. Opening
  // it shows the mail as rows. The line is also a drop target.
  import { slide } from 'svelte/transition'
  import type { AssignRequest } from './AssignLayer.svelte'
  import { dropTarget } from './assignDrag.svelte'
  import type { BundleStore } from './bundleStore.svelte'
  import { reorderable } from './bundleReorder'
  import BundleRows from './BundleRows.svelte'
  import BundleSettings from './BundleSettings.svelte'
  import { bundleTarget, OTHER_KEY, plural, type BundleSectionModel } from './bundles'
  import DragGrip from './DragGrip.svelte'
  import Icon from './Icon.svelte'

  let {
    section,
    store,
    onopen,
    assign,
    ondelete,
    prev = null,
    next = null,
  }: {
    section: BundleSectionModel
    store: BundleStore
    onopen: (id: number) => void
    assign: (req: AssignRequest) => void
    /** Moves mail to the trash with an undo bar. */
    ondelete: (ids: number[], label: string) => void
    /** Neighbouring bundles on screen, for "Nach oben" / "Nach unten"; `null` at the ends. */
    prev?: string | null
    next?: string | null
  } = $props()

  let open = $state(false)
  /** "Ältere löschen …" waiting for its one-line confirmation. */
  let pending = $state<{ ids: number[]; skipped: number } | null>(null)
  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const DAY_S = 86_400

  /** Protected mail is never part of a bulk delete; it is counted so the line can say so. */
  function askOlder(days: number | null) {
    const cutoff = days == null ? Infinity : Date.now() / 1000 - days * DAY_S
    const rows = section.allRows.filter((r) => r.date < cutoff)
    const ids: number[] = []
    let skipped = 0
    for (const r of rows) {
      if (store.placements.get(r.message.id)?.protected) skipped += 1
      else ids.push(r.message.id)
    }
    pending = { ids, skipped }
  }

  function confirmOlder() {
    const p = pending
    pending = null
    if (!p || p.ids.length === 0) return
    ondelete(p.ids, p.ids.length === 1 ? 'Gelöscht' : `${p.ids.length} Mails gelöscht`)
  }

  const panelId = $derived(`bundle-${section.bundle.key}`)
</script>

<section class="bundle" aria-label={section.bundle.name}>
  <div
    class="bundle-line"
    class:open
    use:dropTarget={bundleTarget(section.bundle.key)}
    use:reorderable={{ onreorder: (anchor, side) => void store.reorder(section.bundle.key, anchor, side) }}
  >
    {#if section.bundle.key !== OTHER_KEY}<DragGrip bundle />{/if}
    <button
      type="button"
      class="toggle"
      aria-expanded={open}
      aria-controls={open ? panelId : undefined}
      onclick={() => (open = !open)}
    >
      <span class="chev"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={15} /></span>
      <span class="name">{section.bundle.name}</span>
      <span class="dot" aria-hidden="true">·</span>
      <span class="sentence">{section.sentence}</span>
    </button>
    <BundleSettings bundle={section.bundle} {store} onolder={askOlder} {prev} {next} />
  </div>
  {#if pending}
    <div class="confirm" role="alert">
      {#if pending.ids.length === 0}
        <span class="c-text">
          Nichts zu löschen{pending.skipped > 0
            ? ` · ${pending.skipped} ${plural(pending.skipped, 'geschützte bleibt', 'geschützte bleiben')}`
            : ''}
        </span>
        <button type="button" class="c-btn" onclick={() => (pending = null)}>Schließen</button>
      {:else}
        <span class="c-text">
          {pending.ids.length} {plural(pending.ids.length, 'Mail', 'Mails')} in den Papierkorb verschieben?
          {#if pending.skipped > 0}
            · {pending.skipped} {plural(pending.skipped, 'geschützte bleibt', 'geschützte bleiben')}
          {/if}
        </span>
        <button type="button" class="c-btn danger" onclick={confirmOlder}>Löschen</button>
        <button type="button" class="c-btn" onclick={() => (pending = null)}>Abbrechen</button>
      {/if}
    </div>
  {/if}
  {#if open}
    <div id={panelId} class="list" transition:slide={{ duration: reduceMotion ? 0 : 160 }}>
      <BundleRows
        rows={section.rows}
        bundleKey={section.bundle.key}
        bundles={store.bundles}
        {onopen}
        {assign}
      />
    </div>
  {/if}
</section>

<style>
  .bundle {
    margin-bottom: 4px;
  }
  .bundle-line {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 6px;
    border-radius: 12px;
    transition: background 0.16s ease;
  }
  .bundle-line:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 7px;
    padding: 10px 10px 10px 18px;
    border: 0;
    border-radius: 12px;
    background: none;
    text-align: left;
    font-size: 14px;
    color: var(--text);
  }
  .chev {
    flex: none;
    align-self: center;
    display: inline-flex;
    color: var(--text-soft);
  }
  .name {
    flex: none;
    font-weight: 650;
  }
  .dot {
    flex: none;
    color: var(--text-soft);
  }
  .sentence {
    flex: 1;
    min-width: 0;
    color: var(--text-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    margin: 2px 0 6px 14px;
    padding: 6px 8px 6px 12px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--danger) 9%, var(--card-surface));
    font-size: 13.5px;
  }
  .c-text {
    flex: 1 1 220px;
    min-width: 0;
  }
  .c-btn {
    padding: 5px 12px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .c-btn:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
  }
  .c-btn.danger {
    color: var(--danger);
  }
  .list {
    margin: 2px 0 8px 14px;
    padding: 4px;
    border-radius: var(--card-radius-sm);
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
  }
  @media (prefers-reduced-motion: reduce) {
    .bundle-line {
      transition: none;
    }
  }
</style>

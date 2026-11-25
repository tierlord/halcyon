<script lang="ts">
  // The mail of a bundle as quiet rows (sender · subject · time). Each row
  // opens its mail, can be dragged, and has the "Wohin gehört das?" menu.
  import { slide } from 'svelte/transition'
  import { assignable, selectedTextIn, type AssignChoice } from './assignDrag.svelte'
  import type { AssignRequest } from './AssignLayer.svelte'
  import AssignMenu from './AssignMenu.svelte'
  import DragGrip from './DragGrip.svelte'
  import { bundleTarget, rowTime, type Bundle, type BundleRow, type Target } from './bundles'
  import { useBrand } from './brandView.svelte'

  let {
    rows,
    bundleKey,
    bundles,
    limit = 20,
    onopen,
    assign,
  }: {
    rows: BundleRow[]
    /** The bundle these rows live in (their current placement). */
    bundleKey: string
    bundles: Bundle[]
    /** Rows shown before "Ältere anzeigen". */
    limit?: number
    onopen: (id: number) => void
    assign: (req: AssignRequest) => void
  } = $props()

  const brand = useBrand()
  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const current: Target = $derived(bundleTarget(bundleKey))
  // svelte-ignore state_referenced_locally
  let shown = $state(limit)
  const visible = $derived(rows.slice(0, shown))

  function choose(row: BundleRow, target: AssignChoice, button: HTMLElement) {
    assign({
      messageId: row.message.id,
      target,
      current,
      sourceEl: button.closest<HTMLElement>('[data-assignable]'),
      button,
      name: row.sender,
      brandEmail: row.brandEmail,
      style: brand.style(row.brandEmail, row.sender),
    })
  }
</script>

<ul class="rows">
  {#each visible as row (row.message.id)}
    {@const style = brand.style(row.brandEmail, row.sender)}
    <li
      class="row brand-scope"
      class:new={row.isNew}
      {style}
      data-assignable
      use:assignable={{
        messageId: row.message.id,
        current,
        label: `${row.sender}: ${row.subject}`,
        brandEmail: row.brandEmail,
        name: row.sender,
        style,
      }}
      transition:slide={{ duration: reduceMotion ? 0 : 160 }}
    >
      <DragGrip />
      <button
        type="button"
        class="open"
        onclick={(e) => !selectedTextIn(e.currentTarget) && onopen(row.message.id)}
      >
        <span class="mark" aria-hidden="true"></span>
        <span class="sender">{row.sender}</span>
        <span class="dot" aria-hidden="true">·</span>
        <span class="subject">{row.subject}</span>
        <span class="time">{rowTime(row)}</span>
        {#if row.isNew}<span class="visually-hidden">, neu</span>{/if}
      </button>
      <AssignMenu
        {current}
        {bundles}
        label={`Wohin gehört „${row.subject}“?`}
        onchoose={(t, b) => choose(row, t, b)}
      />
    </li>
  {/each}
</ul>
{#if rows.length > shown}
  <button type="button" class="older" onclick={() => (shown += limit)}>Ältere anzeigen</button>
{/if}

<style>
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    padding-right: 6px;
    padding-left: 14px;
    border-radius: 10px;
    transition: background 0.16s ease;
  }
  .row:hover,
  .row:focus-within {
    background: color-mix(in srgb, var(--brand-acc, var(--text)) 9%, transparent);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 7px;
    padding: 8px 10px;
    border: 0;
    border-radius: 10px;
    background: none;
    text-align: left;
    font-size: 14px;
    cursor: pointer;
    user-select: text;
    -webkit-user-select: text;
  }
  /* New since the last visit: a quiet dot and slightly stronger text. The
     dot keeps its place on every row so the text lines up. */
  .mark {
    flex: none;
    align-self: center;
    width: 6px;
    height: 6px;
    margin-right: -2px;
    border-radius: 50%;
  }
  .row.new .mark {
    background: var(--brand-acc, var(--accent-fill));
  }
  .row.new .sender {
    font-weight: 700;
  }
  .row.new .subject {
    color: var(--text);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .sender {
    flex: none;
    max-width: 38%;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    color: var(--text-soft);
  }
  .subject {
    flex: 1;
    min-width: 0;
    color: var(--text-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .time {
    flex: none;
    font-size: 12.5px;
    color: var(--text-soft);
  }
  .older {
    margin: 4px 0 2px 10px;
    padding: 5px 12px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 13px;
    font-weight: 600;
    color: var(--accent-text);
  }
  .older:hover {
    background: var(--hover);
  }
  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
  }
</style>

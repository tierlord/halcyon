<script lang="ts" module>
  /** What the scene grows from: the card, row or tile that was assigned. */
  export interface SceneSource {
    el: HTMLElement | null
    /** Where it was when the scene started (viewport pixels). */
    rect: DOMRect
    name: string
    brandEmail: string | null
    /** Inline `--brand*` properties. */
    style: string
    /** What gets focus when the scene closes. */
    focusEl: HTMLElement | null
  }
</script>

<script lang="ts">
  // The preview scene of "Wohin gehört das?" (docs/design/relevanz.md, "Wie es
  // aussieht"): the view steps back, the mail stays lit, the kinds of mail of
  // its sender fly out of it as small stacks, and the user picks which ones
  // the assignment covers before anything is written.
  import { onMount, tick, untrack } from 'svelte'
  import { crossfade, fade } from 'svelte/transition'
  import { cubicOut } from 'svelte/easing'
  import Icon from './Icon.svelte'
  import { becauseClause, dateRange, plural, type AssignmentPreview, type PreviewGroup } from './bundles'
  import { listDate } from './format'
  import type { MessageSummary } from './types'

  let {
    source,
    messageId,
    heading,
    preview,
    error = null,
    findMails,
    flyTarget,
    onapply,
    onclose,
  }: {
    source: SceneSource
    messageId: number
    /** "In ‚Werbung' einordnen". */
    heading: string
    /** `null` while the preview is loading. */
    preview: AssignmentPreview | null
    /** A failure to load the preview. */
    error?: string | null
    /** Subjects and dates of mails (for the fanned-out stacks). */
    findMails: (ids: number[]) => Promise<MessageSummary[]>
    /** Where the stacks glide to on "Übernehmen" (the bundle line or the heading). */
    flyTarget: () => DOMRect | null
    /** Writes the assignment; rejects with a German message on failure. */
    onapply: (kindKeys: string[], excludedIds: number[], alsoThisMail: boolean) => Promise<void>
    /** After the scene has glided away. */
    onclose: (applied: boolean) => void
  } = $props()

  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const EASE = 'cubic-bezier(0.2, 0.8, 0.2, 1)'
  const IN_MS = 260
  const OUT_MS = 220
  const STAGGER = 40
  const FAN_LIMIT = 24
  const ASIDE_LIMIT = 8
  const titleId = `scene-${Math.random().toString(36).slice(2, 8)}`

  // ----- state --------------------------------------------------------------------

  let phase = $state<'in' | 'idle' | 'out'>('in')
  let dimmed = $state(false)
  let selected = $state<Set<string>>(new Set())
  let excluded = $state<Set<number>>(new Set())
  let fanned = $state<string | null>(null)
  let mails = $state<Map<number, MessageSummary>>(new Map())
  let landed = $state<boolean[]>([])
  let busy = $state(false)
  let failure = $state<string | null>(null)
  let showAllAside = $state(false)

  let stageEl: HTMLElement | undefined = $state()
  let stageStyle = $state('visibility: hidden')
  let stackEls: (HTMLElement | undefined)[] = $state([])
  let cloneSlot: HTMLElement | undefined = $state()

  const [send, receive] = crossfade({
    duration: reduceMotion ? 0 : 260,
    easing: cubicOut,
    fallback() {
      return {
        duration: reduceMotion ? 0 : 180,
        easing: cubicOut,
        css: (t) => `opacity: ${t}; transform: ${reduceMotion ? 'none' : `scale(${0.92 + 0.08 * t})`}`,
      }
    },
  })

  // ----- derived --------------------------------------------------------------------

  const groups = $derived(preview?.groups ?? [])
  const selectedGroups = $derived(groups.filter((g) => selected.has(g.kind_key)))
  const inSelected = $derived(new Set(selectedGroups.flatMap((g) => g.message_ids)))
  const takenCount = $derived(selectedGroups.reduce((n, g) => n + g.message_ids.filter((id) => !excluded.has(id)).length, 0))
  const protectedShown = $derived(selectedGroups.flatMap((g) => g.protected))
  const mineExcluded = $derived([...excluded].filter((id) => inSelected.has(id)))
  const summary = $derived.by(() => {
    if (!preview) return ''
    if (selectedGroups.length === 0) return 'Nur diese eine Mail wird eingeordnet.'
    const k = selectedGroups.length
    let s = `Gilt für ${takenCount} ${plural(takenCount, 'Mail', 'Mails')} in ${k} ${plural(k, 'Gruppe', 'Gruppen')}.`
    const p = protectedShown.length
    if (p > 0) {
      s += ` ${p} ${plural(p, 'bleibt', 'bleiben')} sichtbar, ${becauseClause(protectedShown.flatMap((x) => x.reasons))}.`
    }
    if (mineExcluded.length > 0) {
      s += ` Du hast ${mineExcluded.length} ${plural(mineExcluded.length, 'Mail', 'Mails')} ausgenommen.`
    }
    return s
  })
  const fanGroup = $derived(groups.find((g) => g.kind_key === fanned) ?? null)
  const fanIds = $derived(fanGroup ? fanGroup.message_ids.filter((id) => !excluded.has(id)) : [])
  const asideRows = $derived(showAllAside ? protectedShown : protectedShown.slice(0, ASIDE_LIMIT))
  const everyoneLanded = $derived(groups.length > 0 && landed.length === groups.length && landed.every(Boolean))
  /** The assignment of this very mail is explicit even if a protection holds it back in a kind. */
  const ownProtected = $derived(groups.some((g) => g.protected.some((p) => p.message_id === messageId)))

  // ----- placement ------------------------------------------------------------------

  function placeStage() {
    if (!stageEl) return
    const vw = window.innerWidth
    const vh = window.innerHeight
    const w = Math.min(Math.max(source.rect.width, 600), 780, vw - 32)
    const left = Math.max(16, Math.min(source.rect.left, vw - w - 16))
    const natural = Math.max(stageEl.scrollHeight, 380)
    const below = vh - source.rect.bottom - 12 - 16
    const above = source.rect.top - 12 - 16
    if (below >= Math.min(natural, 460) || (below >= above && below >= 280)) {
      stageStyle = `left:${left}px;top:${source.rect.bottom + 12}px;width:${w}px;max-height:${Math.max(below, 240)}px`
    } else if (above >= 280) {
      stageStyle = `left:${left}px;bottom:${vh - source.rect.top + 12}px;width:${w}px;max-height:${above}px`
    } else {
      // The source fills the view: the scene lies over it, centred.
      const h = Math.min(natural, vh - 32)
      stageStyle = `left:${left}px;top:${Math.max(16, (vh - h) / 2)}px;width:${w}px;max-height:${vh - 32}px`
    }
  }

  /** A copy of the source on top of the dimmed view: it stays lit where it was. */
  function lightSource() {
    if (!source.el || !cloneSlot) return
    const copy = source.el.cloneNode(true) as HTMLElement
    for (const n of [copy, ...copy.querySelectorAll<HTMLElement>('*')]) {
      n.removeAttribute('id')
      n.removeAttribute('data-drop')
      n.removeAttribute('data-assignable')
      n.removeAttribute('data-dragging')
      n.removeAttribute('data-card')
    }
    copy.querySelectorAll('.mail, iframe, .assign-btn').forEach((n) => n.remove())
    copy.setAttribute('aria-hidden', 'true')
    copy.setAttribute('inert', '')
    copy.classList.add('lit')
    const r = source.rect
    copy.style.cssText += `;position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;max-height:${Math.min(r.height, 420)}px;overflow:hidden;margin:0;pointer-events:none`
    cloneSlot.replaceChildren(copy)
  }

  // ----- fly in -----------------------------------------------------------------------

  const srcCenter = () => ({ x: source.rect.left + source.rect.width / 2, y: source.rect.top + source.rect.height / 2 })

  async function flyIn() {
    landed = groups.map(() => false)
    selected = new Set(groups.filter((g) => g.preselected).map((g) => g.kind_key))
    await tick()
    placeStage()
    await tick()
    phase = 'idle'
    if (reduceMotion) {
      landed = groups.map(() => true)
      focusPrimary()
      return
    }
    const c = srcCenter()
    stackEls.forEach((el, i) => {
      if (!el) return
      const r = el.getBoundingClientRect()
      const dx = c.x - (r.left + r.width / 2)
      const dy = c.y - (r.top + r.height / 2)
      const a = el.animate(
        [
          { transform: `translate(${dx}px, ${dy}px) scale(0.3)`, opacity: 0 },
          { transform: 'none', opacity: 1 },
        ],
        { duration: IN_MS, delay: i * STAGGER, easing: EASE, fill: 'backwards' },
      )
      // Not clickable while still flying (design/interaktion.md).
      a.finished.then(() => {
        landed[i] = true
        if (landed.every(Boolean)) focusPrimary()
      }, () => {})
    })
  }

  function focusPrimary() {
    // Only when the user has not moved on already.
    if (!stageEl || (document.activeElement && document.activeElement !== stageEl && stageEl.contains(document.activeElement))) return
    stageEl.querySelector<HTMLButtonElement>('button.primary')?.focus()
  }

  // ----- fly out ------------------------------------------------------------------------

  /** Stacks glide to `to` (a rect or the source), then the scene is gone. */
  async function leave(to: DOMRect | null, applied: boolean) {
    phase = 'out'
    dimmed = false
    const vw = window.innerWidth
    const vh = window.innerHeight
    const cx = to ? to.left + to.width / 2 : applied ? vw / 2 : srcCenter().x
    const cy = to ? to.top + to.height / 2 : applied ? vh - 24 : srcCenter().y
    let total = reduceMotion ? 0 : OUT_MS
    if (!reduceMotion) {
      const live = stackEls.filter((el, i): el is HTMLElement => !!el && (!applied || selected.has(groups[i]?.kind_key)))
      live.forEach((el, i) => {
        const r = el.getBoundingClientRect()
        const dx = cx - (r.left + r.width / 2)
        const dy = cy - (r.top + r.height / 2)
        el.animate(
          [
            { transform: 'none', opacity: 1 },
            { transform: `translate(${dx}px, ${dy}px) scale(0.25)`, opacity: 0 },
          ],
          { duration: OUT_MS, delay: i * 30, easing: 'cubic-bezier(0.4, 0, 0.8, 0.4)', fill: 'forwards' },
        )
      })
      total = OUT_MS + Math.max(0, live.length - 1) * 30
    }
    await new Promise((r) => setTimeout(r, total + 20))
    onclose(applied)
  }

  function cancel() {
    if (phase === 'out' || busy) return
    void leave(null, false)
  }

  async function confirm() {
    if (phase !== 'idle' || busy || !everyoneLanded) return
    busy = true
    failure = null
    try {
      await onapply([...selected], [...excluded], ownProtected && selected.size > 0)
    } catch (e) {
      failure = String(e)
      busy = false
      return
    }
    await leave(flyTarget(), true)
  }

  // ----- interaction ----------------------------------------------------------------------

  function toggle(g: PreviewGroup, i: number) {
    if (!landed[i] || phase !== 'idle') return
    const next = new Set(selected)
    if (next.has(g.kind_key)) next.delete(g.kind_key)
    else next.add(g.kind_key)
    selected = next
  }

  async function toggleFan(g: PreviewGroup, i: number) {
    if (!landed[i] || phase !== 'idle') return
    if (fanned === g.kind_key) {
      fanned = null
      return
    }
    fanned = g.kind_key
    const missing = g.message_ids.slice(0, FAN_LIMIT + excluded.size).filter((id) => !mails.has(id))
    if (missing.length > 0) {
      try {
        const found = await findMails(missing)
        const next = new Map(mails)
        for (const m of found) next.set(m.id, m)
        mails = next
      } catch {
        // The chips fall back to "Mail".
      }
    }
    await tick()
    placeStage()
  }

  function exclude(id: number) {
    excluded = new Set(excluded).add(id)
  }

  function include(id: number) {
    const next = new Set(excluded)
    next.delete(id)
    excluded = next
  }

  /** A short pull out of the fan counts as "not this one". */
  function flick(node: HTMLElement, id: number) {
    let start: { x: number; y: number } | null = null
    const down = (e: PointerEvent) => {
      if (e.button !== 0 || (e.target as Element).closest('button')) return
      start = { x: e.clientX, y: e.clientY }
      node.setPointerCapture(e.pointerId)
    }
    const move = (e: PointerEvent) => {
      if (!start) return
      node.style.transform = `translate(${e.clientX - start.x}px, ${e.clientY - start.y}px)`
      node.style.zIndex = '2'
    }
    const up = (e: PointerEvent) => {
      if (!start) return
      const far = Math.hypot(e.clientX - start.x, e.clientY - start.y) > 44
      start = null
      node.style.transform = ''
      node.style.zIndex = ''
      if (far) exclude(id)
    }
    node.addEventListener('pointerdown', down)
    node.addEventListener('pointermove', move)
    node.addEventListener('pointerup', up)
    node.addEventListener('pointercancel', up)
    return {
      destroy() {
        node.removeEventListener('pointerdown', down)
        node.removeEventListener('pointermove', move)
        node.removeEventListener('pointerup', up)
        node.removeEventListener('pointercancel', up)
      },
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      cancel()
    } else if (e.key === 'Tab' && stageEl) {
      // The scene is modal: Tab stays inside.
      const list = [...stageEl.querySelectorAll<HTMLElement>('button:not(:disabled)')]
      if (list.length === 0) return
      const i = list.indexOf(document.activeElement as HTMLElement)
      const next = e.shiftKey ? (i <= 0 ? list.length - 1 : i - 1) : i === list.length - 1 ? 0 : i + 1
      e.preventDefault()
      list[next].focus()
    }
  }

  function portal(node: HTMLElement) {
    document.body.appendChild(node)
    return { destroy: () => node.remove() }
  }

  onMount(() => {
    lightSource()
    requestAnimationFrame(() => (dimmed = true))
    placeStage()
    stageEl?.focus()
    window.addEventListener('keydown', onKeydown, true)
    return () => window.removeEventListener('keydown', onKeydown, true)
  })

  // The preview arrives after the scene opened.
  $effect(() => {
    if (!preview) return
    untrack(() => void flyIn())
  })

  function chipTitle(id: number): string {
    return mails.get(id)?.header.subject ?? 'Mail'
  }
  function chipDate(id: number): string {
    return listDate(mails.get(id)?.header.date ?? null)
  }
</script>

<div use:portal class="scene brand-scope" style={source.style} data-phase={phase}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="dim" class:on={dimmed} onclick={cancel}></div>
  <div class="lit-slot" bind:this={cloneSlot}></div>

  <div
    bind:this={stageEl}
    class="stage"
    class:out={phase === 'out'}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    style={stageStyle}
  >
    <header class="head">
      <h2 id={titleId}>{heading}</h2>
      {#if preview}
        <p class="from">
          Von {preview.sender_name ?? preview.sender}
          · {preview.total_mails} {plural(preview.total_mails, 'Mail', 'Mails')} in {preview.group_count}
          {plural(preview.group_count, 'Gruppe', 'Gruppen')}
        </p>
      {/if}
    </header>

    {#if error}
      <p class="note" role="alert">{error}</p>
    {:else if !preview}
      <p class="note" role="status">Einen Moment …</p>
    {:else}
      <div class="stacks" role="group" aria-label="Arten von Mails dieses Absenders">
        {#each groups as g, i (g.kind_key)}
          {@const on = selected.has(g.kind_key)}
          <div class="slot" bind:this={stackEls[i]}>
            <button
              type="button"
              class="stack"
              class:on
              aria-pressed={on}
              disabled={!landed[i] || phase !== 'idle'}
              onclick={() => toggle(g, i)}
              ondblclick={() => toggleFan(g, i)}
            >
              <span class="pile" aria-hidden="true">
                {#if g.count > 2}<span class="back b2"></span>{/if}
                {#if g.count > 1}<span class="back b1"></span>{/if}
                <span class="top">{g.examples[0] ?? g.label}</span>
              </span>
              <span class="label">
                {g.label}
                {#if on}<span class="tick"><Icon name="check" size={13} /></span>{/if}
              </span>
              <span class="meta">{g.count} {plural(g.count, 'Mail', 'Mails')}</span>
              <span class="meta">{dateRange(g.first_date, g.last_date)}</span>
              {#if g.assigned}<span class="meta assigned">hat schon eine Zuordnung</span>{/if}
            </button>
            <button
              type="button"
              class="fan-btn"
              aria-expanded={fanned === g.kind_key}
              disabled={!landed[i] || phase !== 'idle' || g.message_ids.length === 0}
              onclick={() => toggleFan(g, i)}
            >
              {fanned === g.kind_key ? 'Zufächern' : 'Auffächern'}
            </button>
          </div>
        {/each}
      </div>

      {#if fanGroup}
        <div class="fan" role="group" aria-label={`Mails: ${fanGroup.label}`}>
          {#each fanIds.slice(0, FAN_LIMIT) as id (id)}
            <div
              class="chip"
              use:flick={id}
              in:receive={{ key: id }}
              out:send={{ key: id }}
            >
              <span class="c-main">
                <span class="c-subject">{chipTitle(id)}</span>
                <span class="c-date">{chipDate(id)}</span>
              </span>
              <button type="button" class="x" aria-label={`${chipTitle(id)} ausnehmen`} onclick={() => exclude(id)}>
                <Icon name="x" size={13} />
              </button>
            </div>
          {/each}
          {#if fanIds.length > FAN_LIMIT}
            <p class="more">und {fanIds.length - FAN_LIMIT} weitere</p>
          {/if}
        </div>
      {/if}

      {#if protectedShown.length > 0 || mineExcluded.length > 0}
        <section class="aside" aria-label="Ausnahmen">
          <h3>Ausnahmen <span class="sub">bleiben, wo sie sind</span></h3>
          <ul>
            {#each asideRows as p (p.message_id)}
              <li class="a-row" in:fade={{ duration: reduceMotion ? 0 : 160 }}>
                <span class="a-subject">{p.subject}</span>
                <span class="a-why">{p.reasons.join(' · ')}</span>
              </li>
            {/each}
            {#each mineExcluded as id (id)}
              <li class="a-row mine" in:receive={{ key: id }} out:send={{ key: id }}>
                <span class="a-subject">{chipTitle(id)}</span>
                <span class="a-why">von dir ausgenommen</span>
                <button type="button" class="again" onclick={() => include(id)}>Wieder einbeziehen</button>
              </li>
            {/each}
          </ul>
          {#if protectedShown.length > ASIDE_LIMIT && !showAllAside}
            <button type="button" class="again" onclick={() => (showAllAside = true)}>
              {protectedShown.length - ASIDE_LIMIT} weitere zeigen
            </button>
          {/if}
        </section>
      {/if}

      <p class="summary" aria-live="polite">{summary}</p>
    {/if}

    {#if failure}<p class="note error" role="alert">{failure}</p>{/if}

    <footer class="foot">
      <button type="button" class="btn quiet" onclick={cancel}>Abbrechen</button>
      <button
        type="button"
        class="btn primary"
        disabled={!preview || !everyoneLanded || busy || phase !== 'idle'}
        onclick={confirm}
      >
        Übernehmen
      </button>
    </footer>
  </div>
</div>

<style>
  .scene {
    position: fixed;
    inset: 0;
    z-index: 70;
  }
  .dim {
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--pitch-black) 42%, transparent);
    -webkit-backdrop-filter: blur(3px);
    backdrop-filter: blur(3px);
    opacity: 0;
    transition: opacity 0.2s ease-out;
  }
  .dim.on {
    opacity: 1;
  }
  .lit-slot :global(.lit) {
    box-shadow:
      var(--card-shadow-hover),
      0 0 0 2px var(--brand-acc, var(--highlight)),
      0 18px 44px -12px color-mix(in srgb, var(--pitch-black) 45%, transparent);
    transform: none;
  }
  .lit-slot :global(.lit),
  .lit-slot :global(.lit *) {
    transition: none !important;
    animation: none !important;
  }

  .stage {
    position: fixed;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 20px 16px;
    overflow-y: auto;
    border-radius: var(--card-radius);
    background: var(--surface-raised);
    box-shadow:
      0 24px 60px -12px color-mix(in srgb, var(--pitch-black) 50%, transparent),
      0 0 0 1px color-mix(in srgb, var(--text) 8%, transparent);
    outline: none;
    animation: stage-in 0.22s ease-out;
    transition: opacity 0.2s ease-in;
  }
  .stage.out {
    opacity: 0;
  }
  @keyframes stage-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .head h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 650;
  }
  .from {
    margin: 2px 0 0;
    font-size: 13px;
    color: var(--text-soft);
  }
  .note {
    margin: 0;
    font-size: 14px;
    color: var(--text-soft);
  }
  .note.error {
    color: var(--danger);
  }

  .stacks {
    display: flex;
    gap: 14px;
    padding: 6px 4px 8px;
    margin: 0 -4px;
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .slot {
    flex: none;
    width: 172px;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 12px 12px 12px;
    border: 0;
    border-radius: var(--card-radius-sm);
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
    color: var(--text);
    text-align: left;
    opacity: 0.8;
    transition: opacity 0.14s ease, box-shadow 0.14s ease;
  }
  .stack:disabled {
    color: var(--text);
  }
  .stack.on {
    opacity: 1;
    background: linear-gradient(160deg, var(--tint), var(--tint-end) 80%), var(--card-surface);
    box-shadow:
      var(--card-shadow),
      var(--card-highlight),
      0 0 0 2px var(--brand-acc, var(--highlight));
  }
  .stack:not(:disabled):hover {
    opacity: 1;
  }
  .pile {
    position: relative;
    display: block;
    height: 64px;
    margin-bottom: 8px;
  }
  .back,
  .top {
    position: absolute;
    left: 0;
    right: 0;
    border-radius: 10px;
    background: var(--surface-raised);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--text) 10%, transparent);
  }
  .back.b1 {
    top: 6px;
    height: 56px;
    transform: translateX(4px) rotate(1.8deg);
    opacity: 0.8;
  }
  .back.b2 {
    top: 10px;
    height: 54px;
    transform: translateX(-3px) rotate(-2.2deg);
    opacity: 0.55;
  }
  .top {
    top: 0;
    height: 58px;
    padding: 8px 10px;
    font-size: 12px;
    line-height: 1.35;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
    font-weight: 650;
    line-height: 1.3;
  }
  .tick {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 99px;
    background: var(--brand-acc, var(--accent-fill));
    color: var(--pitch-black);
    flex: none;
  }
  .meta {
    font-size: 12px;
    color: var(--text-soft);
    line-height: 1.35;
  }
  .meta.assigned {
    font-style: italic;
  }
  .fan-btn {
    align-self: flex-start;
    padding: 3px 10px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--accent-text);
  }
  .fan-btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .fan-btn:disabled {
    visibility: hidden;
  }

  .fan {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px;
    padding: 10px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--text) 5%, transparent);
  }
  .chip {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 8px 12px;
    border-radius: 12px;
    background: var(--surface-raised);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--text) 10%, transparent);
    touch-action: none;
    cursor: grab;
    user-select: none;
  }
  .c-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .c-subject {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-date {
    font-size: 11.5px;
    color: var(--text-soft);
  }
  .x {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 99px;
    background: none;
    color: var(--text-soft);
  }
  .x:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  .more {
    margin: 0;
    align-self: center;
    font-size: 12.5px;
    color: var(--text-soft);
  }

  .aside {
    padding: 10px 12px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--ochre) 8%, var(--surface-raised));
  }
  .aside h3 {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 650;
  }
  .aside .sub {
    font-weight: 400;
    color: var(--text-soft);
  }
  .aside ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .a-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 10px;
    font-size: 13px;
  }
  .a-subject {
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .a-why {
    font-size: 12px;
    color: var(--text-soft);
  }
  .again {
    padding: 2px 8px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent-text);
  }
  .again:hover {
    background: var(--hover);
  }

  .summary {
    margin: 0;
    font-size: 14px;
    line-height: 1.5;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    padding: 8px 18px;
    border: 0;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    font-size: 14px;
    font-weight: 600;
    transition: background 0.16s ease;
  }
  .btn.quiet {
    background: none;
    color: var(--text-soft);
    font-weight: 500;
  }
  .btn.quiet:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  .btn.primary {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (prefers-reduced-motion: reduce) {
    .stage {
      animation: none;
    }
    .dim,
    .stack,
    .stage {
      transition-duration: 0.01s;
    }
  }
</style>

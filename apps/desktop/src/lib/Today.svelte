<script lang="ts">
  // "Heute": the start view. It answers "what is new that matters to me?"
  // from a ready-made model (see today.ts); actions go back to App.svelte.
  import { onMount, tick } from 'svelte'
  import { slide } from 'svelte/transition'
  import Icon from './Icon.svelte'
  import ReadingPane from './ReadingPane.svelte'
  import { brandStyle, logoColor } from './brandColor'
  import { extractBrandDomain, getBrandLogo, subscribeLogos } from './brandLogos'
  import { initials } from './format'
  import { ORDER_STEPS, type Dismissal, type ForYouItem, type OrderGroup, type OrderItem, type TodayModel } from './today'
  import type { MessageSummary } from './types'

  let {
    model,
    undo = null,
    expandedKey = $bindable(null),
    findMessage,
    onexpand,
    ontoggleread,
    ontoggleflag,
    ondelete,
    onreplyto,
    onopen,
    onreply,
    ondismiss,
    onundo,
  }: {
    model: TodayModel
    /** The pending "Entfernt · Rückgängig" bar; null when nothing can be undone. */
    undo?: { label: string } | null
    /** Key of the expanded "Für dich" card; owned by the parent so it survives the full view. */
    expandedKey?: string | null
    /** Looks a mail up in the app's pool; null when it is gone. */
    findMessage: (id: number) => MessageSummary | null
    /** A mail was opened inside a card (the parent marks it as read). */
    onexpand: (id: number) => void
    ontoggleread: (m: MessageSummary) => void
    ontoggleflag: (m: MessageSummary) => void
    ondelete: (m: MessageSummary) => void
    onreplyto: (m: MessageSummary, kind: 'reply' | 'reply_all' | 'forward') => void
    /** Opens the full reading view. */
    onopen: (id: number) => void
    onreply: (m: MessageSummary) => void
    /** Removes an item; `label` overrides the text of the undo bar. */
    ondismiss: (d: Dismissal, label?: string) => void
    onundo: () => void
  } = $props()

  let logoVersion = $state(0)
  let asUsualOpen = $state(false)
  let root: HTMLElement | undefined = $state()
  /** Company key of the order group whose details are open below the strip. */
  let openGroupKey = $state<string | null>(null)
  /** Case id of the expanded order inside a group of three or more (accordion). */
  let openOrderId = $state<number | null>(null)
  const openGroup = $derived(model.orderGroups.find((g) => g.key === openGroupKey) ?? null)

  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const COLLAPSE_MS = reduceMotion ? 0 : 200
  const EXPAND_MS = reduceMotion ? 0 : 160
  const CARD_MS = reduceMotion ? 0 : 260
  /** Layers shown for a stack of orders: the front tile plus up to two behind. */
  const MAX_LAYERS = 3

  onMount(() => {
    const unsubscribe = subscribeLogos(() => (logoVersion += 1))
    // Coming back from the full reading view: bring the open card into sight.
    if (expandedKey) void tick().then(() => scrollCardIntoView(expandedKey!, 'auto'))
    return unsubscribe
  })

  function logoFor(email: string | null) {
    void logoVersion
    return email ? getBrandLogo(email) : null
  }

  /**
   * Inline `--brand*` properties for a card: dominant logo colour, else the
   * logo tile colour, else a hue derived from the sender (see brandColor.ts).
   */
  function brandFor(email: string | null, name: string): string {
    const logo = logoFor(email)
    const sampled = logo ? logoColor(logo.dataUrl, () => (logoVersion += 1)) : null
    return brandStyle(sampled ?? logo?.bg ?? null, extractBrandDomain(email) ?? name)
  }

  function orderBrand(o: OrderItem) {
    return o.caseItem.domain ? `info@${o.caseItem.domain}` : null
  }

  function markArrived(o: OrderItem) {
    ondismiss(
      { kind: 'case', caseId: o.caseItem.id, previous: o.caseItem.status },
      'Als angekommen markiert',
    )
  }

  // ----- "Für dich": expand in place -----------------------------------------

  function scrollCardIntoView(key: string, behavior: ScrollBehavior = reduceMotion ? 'auto' : 'smooth') {
    root?.querySelector(`[data-card="${CSS.escape(key)}"]`)?.scrollIntoView({ block: 'nearest', behavior })
  }

  async function toggleCard(item: ForYouItem) {
    if (item.openId == null) return
    if (expandedKey === item.key) {
      expandedKey = null
      return
    }
    expandedKey = item.key
    onexpand(item.openId)
    await tick()
    // after the grow animation, so the final size decides the scroll
    setTimeout(() => expandedKey === item.key && scrollCardIntoView(item.key), CARD_MS + 40)
  }

  /** The whole card is the click target, except real controls and the mail itself. */
  function onCardClick(e: MouseEvent, item: ForYouItem) {
    if ((e.target as Element).closest('button, a, input, iframe, .mail')) return
    void toggleCard(item)
  }

  function onHeadKey(e: KeyboardEvent, item: ForYouItem) {
    if (e.key !== 'Enter' && e.key !== ' ') return
    e.preventDefault()
    void toggleCard(item)
  }

  function dismissCard(item: ForYouItem) {
    if (expandedKey === item.key) expandedKey = null
    ondismiss(item.dismissal)
  }

  // ----- Orders strip ----------------------------------------------------------

  let stripEl: HTMLElement | undefined = $state()
  let fadeLeft = $state(false)
  let fadeRight = $state(false)

  function updateFade() {
    if (!stripEl) return
    fadeLeft = stripEl.scrollLeft > 2
    fadeRight = stripEl.scrollLeft + stripEl.clientWidth < stripEl.scrollWidth - 2
  }

  $effect(() => {
    void model.orderGroups.length
    if (!stripEl) return
    void tick().then(updateFade)
    const ro = new ResizeObserver(updateFade)
    ro.observe(stripEl)
    return () => ro.disconnect()
  })

  /** Vertical wheel / trackpad movement scrolls the strip sideways. */
  function wheelX(node: HTMLElement) {
    const handler = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return
      const max = node.scrollWidth - node.clientWidth
      if (max <= 0) return
      if ((e.deltaY < 0 && node.scrollLeft <= 0) || (e.deltaY > 0 && node.scrollLeft >= max)) return
      e.preventDefault()
      node.scrollLeft = Math.max(0, Math.min(max, node.scrollLeft + e.deltaY))
    }
    node.addEventListener('wheel', handler, { passive: false })
    return { destroy: () => node.removeEventListener('wheel', handler) }
  }

  function onStripKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
    const tiles = [...(stripEl?.querySelectorAll<HTMLButtonElement>('.tile') ?? [])]
    const i = tiles.indexOf(document.activeElement as HTMLButtonElement)
    const next = tiles[i + (e.key === 'ArrowRight' ? 1 : -1)]
    if (i < 0 || !next) return
    e.preventDefault()
    next.focus()
  }

  function toggleGroup(g: OrderGroup) {
    openOrderId = null
    openGroupKey = openGroupKey === g.key ? null : g.key
  }

  function closeGroup(refocus: boolean) {
    const key = openGroupKey
    openGroupKey = null
    if (refocus && key) root?.querySelector<HTMLElement>(`[data-group="${CSS.escape(key)}"]`)?.focus()
  }

  function fillDash(step: number): string {
    // pathLength is 100: four segments of 20 with 5 gaps; unreached ones are skipped
    return `${'20 5 '.repeat(step + 1)}0 100`
  }

  function groupStatus(g: OrderGroup): string {
    return g.orders.length === 1 ? g.orders[0].chip : `${g.orders.length} Bestellungen`
  }

  function groupLabel(g: OrderGroup): string {
    if (g.orders.length !== 1) return g.name
    const title = g.orders[0].caseItem.title
    // Without a recognised article the title is only "Bestellung 123" or
    // "Sendung 123", which truncates to a meaningless "Bestellung …".
    return /^(bestellung|sendung)\b/i.test(title) ? g.name : title
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    const t = e.target as Element | null
    if (t?.closest?.('[role="alertdialog"]')) return
    // only react while focus is in this view (or nowhere), not in the composer etc.
    if (t && t !== document.body && !root?.contains(t)) return
    if (t?.closest?.('.foryou') && expandedKey) {
      expandedKey = null
    } else if (openGroupKey) {
      closeGroup(!!t?.closest?.('.strip-block'))
    } else if (expandedKey) {
      expandedKey = null
    } else return
    e.preventDefault()
  }
</script>

<svelte:window onkeydown={onWindowKey} />

{#snippet avatar(email: string | null, name: string)}
  {@const logo = logoFor(email)}
  <span class="avatar" class:has-logo={!!logo} style:--logo-bg={logo?.bg ?? 'transparent'} aria-hidden="true">
    {#if logo}
      <img src={logo.dataUrl} alt="" class:opaque={!logo.padded} />
    {:else}
      {initials(name, email)}
    {/if}
  </span>
{/snippet}

{#snippet dismissButton(d: Dismissal)}
  <button type="button" class="dismiss" aria-label="Entfernen" onclick={() => ondismiss(d)}>
    <Icon name="x" size={15} />
  </button>
{/snippet}

{#snippet progress(o: OrderItem)}
  <span class="progress" role="img" aria-label={`Fortschritt: ${o.chip}`}>
    <span class="track">
      <span class="fill" style:width={`${(o.step / (ORDER_STEPS.length - 1)) * 100}%`}></span>
      {#each ORDER_STEPS as _, i}
        <span class="dot" class:reached={i <= o.step} style:left={`${(i / (ORDER_STEPS.length - 1)) * 100}%`}></span>
      {/each}
    </span>
    <span class="steps" aria-hidden="true">
      {#each ORDER_STEPS as label, i}
        <span class="step" class:current={i === o.step}>{label}</span>
      {/each}
    </span>
  </span>
{/snippet}

{#snippet orderHead(o: OrderItem)}
  {@render avatar(orderBrand(o), o.caseItem.counterparty)}
  <span class="o-texts">
    <span class="o-shop">{o.caseItem.counterparty}</span>
    <span class="o-title">{o.caseItem.title}</span>
  </span>
  <span class="chip step-{o.step}">{o.chip}</span>
  {@render progress(o)}
{/snippet}

<!-- One order with its details. `forceOpen`: no accordion (single order or at most two in a stack). -->
{#snippet orderBlock(o: OrderItem, idx: number, forceOpen: boolean)}
  {@const open = forceOpen || openOrderId === o.caseItem.id}
  <li
    class="order brand-scope"
    class:open
    class:arrived={o.arrived}
    style={`${brandFor(orderBrand(o), o.caseItem.counterparty)};--i:${Math.min(idx, 6)}`}
  >
    {#if forceOpen}
      <div class="order-head static">{@render orderHead(o)}</div>
    {:else}
      <button
        type="button"
        class="order-head"
        aria-expanded={open}
        aria-controls={`order-${o.caseItem.id}`}
        onclick={() => (openOrderId = open ? null : o.caseItem.id)}
      >
        {@render orderHead(o)}
      </button>
    {/if}
    {#if o.hint && !o.arrived}
      <div class="hint-row">
        <span class="hint">{o.hint}</span>
        {#if o.askArrived}
          <button type="button" class="btn small" onclick={() => markArrived(o)}>
            <Icon name="check" size={14} /> Ist angekommen
          </button>
        {/if}
      </div>
    {/if}
    {#if open}
      <div class="order-detail" id={`order-${o.caseItem.id}`} transition:slide={{ duration: EXPAND_MS }}>
        <dl class="facts">
          {#if o.caseItem.amount}
            <div><dt>Betrag</dt><dd>{o.caseItem.amount}</dd></div>
          {/if}
          {#each o.caseItem.keys as k}
            <div><dt>{k.label}</dt><dd>{k.value}</dd></div>
          {/each}
        </dl>
        {#if o.timeline.length > 0}
          <ol class="timeline" aria-label="Verlauf">
            {#each o.timeline as e (e.id)}
              <li>
                <span class="t-dot" aria-hidden="true"></span>
                <span class="t-date">{e.date}</span>
                {#if e.messageId != null}
                  <button type="button" class="t-label link" onclick={() => onopen(e.messageId!)}>
                    {e.label}
                  </button>
                {:else}
                  <span class="t-label">{e.label}</span>
                {/if}
              </li>
            {/each}
          </ol>
        {/if}
        {#if o.caseItem.attachments?.length}
          <ul class="attachments" aria-label="Anhänge">
            {#each o.caseItem.attachments as a (a.id)}
              <li>
                <Icon name="paperclip" size={14} />
                {#if a.messageId != null}
                  <button type="button" class="link" onclick={() => onopen(a.messageId!)}>{a.name}</button>
                {:else}
                  <span>{a.name}</span>
                {/if}
                {#if a.sizeLabel}<span class="muted">{a.sizeLabel}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="actions">
          {#if o.openId != null}
            <button type="button" class="btn small" onclick={() => onopen(o.openId!)}>Mail öffnen</button>
          {/if}
          {#if !o.arrived}
            <button type="button" class="btn small" onclick={() => markArrived(o)}>
              <Icon name="check" size={14} /> Ist angekommen
            </button>
          {/if}
        </div>
      </div>
    {/if}
  </li>
{/snippet}

<div class="today-root" bind:this={root}>
  <div class="today">
    <div class="layout">
      <header class="greeting block">
        <p class="date">{model.dateLine}</p>
        <h1>{model.greeting}</h1>
        <p class="summary">{model.summary}</p>
      </header>

      {#if model.orderGroups.length > 0}
        <section class="strip-block" aria-labelledby="t-orders">
          <h2 id="t-orders" class="sr-only">Unterwegs</h2>
          <div class="strip-wrap" class:fade-left={fadeLeft} class:fade-right={fadeRight}>
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <ul
              class="strip"
              bind:this={stripEl}
              use:wheelX
              onscroll={updateFade}
              onkeydown={onStripKey}
            >
              {#each model.orderGroups as g (g.key)}
                {@const open = openGroupKey === g.key}
                {@const front = g.orders[0]}
                {@const layers = Math.min(g.orders.length, MAX_LAYERS)}
                <li>
                  <button
                    type="button"
                    class="tile brand-scope"
                    class:open
                    class:muted={g.arrived}
                    style={brandFor(g.brandEmail, g.name)}
                    data-group={g.key}
                    aria-expanded={open}
                    aria-controls={open ? 'order-panel' : undefined}
                    aria-label={`${groupLabel(g)}, ${groupStatus(g)}${g.attention ? ', braucht einen Blick' : ''}`}
                    onclick={() => toggleGroup(g)}
                  >
                    <span class="tile-art" style:--extra={layers - 1}>
                      {#each Array.from({ length: layers - 1 }, (_, i) => layers - 1 - i) as d (d)}
                        <span class="layer" style:--d={d} aria-hidden="true"></span>
                      {/each}
                      <svg class="ring" viewBox="0 0 64 64" aria-hidden="true">
                        <circle class="r-track" cx="32" cy="32" r="30" pathLength="100" />
                        <circle class="r-fill" cx="32" cy="32" r="30" pathLength="100" stroke-dasharray={fillDash(front.step)} />
                      </svg>
                      {@render avatar(g.brandEmail, g.name)}
                      {#if g.attention}<span class="attn" aria-hidden="true"></span>{/if}
                    </span>
                    <span class="t-name">{groupLabel(g)}</span>
                    <span class="t-status">{groupStatus(g)}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </div>

          {#if openGroup}
            <div
              class="panel"
              id="order-panel"
              role="region"
              aria-label={`Bestellungen bei ${openGroup.name}`}
              transition:slide={{ duration: EXPAND_MS }}
            >
              {#if openGroup.orders.length > 1}
                <h3 class="panel-title">Deine Bestellungen bei {openGroup.name}</h3>
              {/if}
              <ul class="orders">
                {#each openGroup.orders as o, idx (o.caseItem.id)}
                  {@render orderBlock(o, idx, openGroup.orders.length <= 2)}
                {/each}
              </ul>
            </div>
          {/if}
        </section>
      {/if}

      <div
        class="today-columns"
        class:two-col={model.quiet.length > 0 || model.asUsual !== null}
      >
        <div class="col-main">
      {#if model.forYou.length === 0}
        <section class="block" aria-labelledby="t-foryou">
          <h2 id="t-foryou">Für dich</h2>
          <div class="caught-up">
            <span class="cu-icon" aria-hidden="true"><Icon name="check" size={22} /></span>
            <p>Alles erledigt. Halcyon meldet sich, wenn etwas Neues für dich da ist.</p>
          </div>
        </section>
      {:else}
        <section class="block" aria-labelledby="t-foryou">
          <h2 id="t-foryou">Für dich</h2>
          <div class="cards">
            {#each model.forYou as item, idx (item.key)}
              {@const expanded = expandedKey === item.key}
              {@const canToggle = item.openId != null}
              {@const msg = expanded && item.openId != null ? findMessage(item.openId) : null}
              <!-- Whole card is clickable for mouse users; the header is the keyboard/AT control. -->
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
              <article
                class="card foryou brand-scope"
                class:expanded
                class:can-toggle={canToggle}
                data-card={item.key}
                style={`${brandFor(item.brandEmail, item.name)};--i:${Math.min(idx, 6)}`}
                onclick={(e) => onCardClick(e, item)}
                out:slide={{ duration: COLLAPSE_MS }}
              >
                <div class="card-tools">
                  {#if expanded && msg}
                    <button
                      type="button"
                      class="dismiss"
                      aria-label="Groß anzeigen"
                      title="Groß anzeigen"
                      onclick={() => onopen(item.openId!)}
                    >
                      <span class="fs-icon"><Icon name="forward" size={15} /></span>
                    </button>
                  {/if}
                  <button type="button" class="dismiss" aria-label="Entfernen" onclick={() => dismissCard(item)}>
                    <Icon name="x" size={15} />
                  </button>
                </div>
                <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                <div
                  class="card-body"
                  class:head-toggle={canToggle}
                  role={canToggle ? 'button' : undefined}
                  tabindex={canToggle ? 0 : undefined}
                  aria-expanded={canToggle ? expanded : undefined}
                  aria-controls={canToggle && expanded ? `mail-${item.key}` : undefined}
                  onkeydown={canToggle ? (e) => onHeadKey(e, item) : undefined}
                >
                  {@render avatar(item.brandEmail, item.name)}
                  <div class="texts">
                    <p class="headline" title={item.why}>{item.headline}</p>
                    {#if item.sub}<p class="sub">{item.sub}</p>{/if}
                    {#if item.detail && !expanded}<p class="detail">{item.detail}</p>{/if}
                  </div>
                </div>
                <div class="actions">
                  {#if item.replyTo && !expanded}
                    <button type="button" class="btn primary" onclick={() => onreply(item.replyTo!)}>
                      <Icon name="reply" size={15} /> Antworten
                    </button>
                  {/if}
                  {#if item.openId != null}
                    <button
                      type="button"
                      class="btn"
                      class:primary={!item.replyTo}
                      onclick={() => toggleCard(item)}
                    >
                      {expanded ? 'Zuklappen' : 'Ansehen'}
                    </button>
                  {/if}
                  {#if item.caseId != null}
                    <button type="button" class="btn quiet" onclick={() => dismissCard(item)}>
                      <Icon name="check" size={15} /> Ist erledigt
                    </button>
                  {:else if item.sourceId != null}
                    <button type="button" class="btn quiet" onclick={() => dismissCard(item)}>
                      Verstanden
                    </button>
                  {/if}
                </div>
                {#if expanded}
                  <div class="mail" id={`mail-${item.key}`} transition:slide={{ duration: CARD_MS }}>
                    {#if msg}
                      <ReadingPane
                        embedded
                        message={msg}
                        {ontoggleread}
                        {ontoggleflag}
                        {ondelete}
                        onreply={onreplyto}
                      />
                    {:else}
                      <p class="gone">Diese Mail gibt es nicht mehr. Vielleicht hast du sie schon gelöscht.</p>
                    {/if}
                  </div>
                {/if}
              </article>
            {/each}
          </div>
        </section>
      {/if}
        </div>

      {#if model.quiet.length > 0 || model.asUsual}
        <div class="col-side">
      {#if model.quiet.length > 0}
        <section class="block" aria-labelledby="t-quiet">
          <h2 id="t-quiet">Läuft von allein</h2>
          <ul class="quiet-list">
            {#each model.quiet as q (q.key)}
              <li out:slide={{ duration: COLLAPSE_MS }}>
                <span class="q-icon"><Icon name={q.icon} size={17} /></span>
                <button
                  type="button"
                  class="q-text"
                  disabled={q.openId == null}
                  onclick={() => q.openId != null && onopen(q.openId)}
                >
                  <span class="q-main">{q.text}</span>
                  {#if q.sub}<span class="q-sub">{q.sub}</span>{/if}
                </button>
                {#if q.followUp}
                  <button type="button" class="btn small" onclick={() => onreply(q.followUp!)}>Nachhaken</button>
                {/if}
                {@render dismissButton(q.dismissal)}
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if model.asUsual}
        <section class="block as-usual" aria-label="Wie immer">
          <div class="usual-card">
          <button
            type="button"
            class="usual-toggle"
            aria-expanded={asUsualOpen}
            onclick={() => (asUsualOpen = !asUsualOpen)}
          >
            <span class="chev"><Icon name={asUsualOpen ? 'chevronDown' : 'chevronRight'} size={16} /></span>
            <span>{model.asUsual.summary}</span>
          </button>
          {#if asUsualOpen}
            <ul class="usual-list" transition:slide={{ duration: EXPAND_MS }}>
              {#each model.asUsual.rows as r (r.message.id)}
                <li>
                  <button type="button" class="usual-row" onclick={() => onopen(r.message.id)}>
                    <span class="u-sender">{r.sender}</span>
                    <span class="u-dot" aria-hidden="true">·</span>
                    <span class="u-subject">{r.subject}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
          </div>
        </section>
      {/if}
        </div>
      {/if}
      </div>
    </div>
  </div>

  {#if undo}
    <div class="undo-bar" role="status">
      <span>{undo.label}</span>
      <span aria-hidden="true">·</span>
      <button type="button" class="undo-btn" onclick={onundo}>Rückgängig</button>
    </div>
  {/if}
</div>

<style>
  .today-root {
    position: relative;
    height: 100%;
    min-height: 0;
  }
  .today {
    height: 100%;
    overflow-y: auto;
    padding: 44px 24px 96px;
    container-type: inline-size;
  }

  /* One centred column. Spacing is margin-based so absent sections leave
     no empty grid rows behind. */
  .layout {
    max-width: 820px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
  }
  .today-columns {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .col-main,
  .col-side {
    min-width: 0;
  }
  @container (min-width: 1060px) {
    .layout:has(.today-columns.two-col) {
      max-width: 1160px;
    }
    .today-columns.two-col {
      display: grid;
      grid-template-columns: minmax(0, 1.35fr) minmax(300px, 0.85fr);
      gap: 40px;
      align-items: start;
    }
    .col-side {
      position: sticky;
      top: 0;
    }
  }
  .block {
    margin-bottom: 40px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .greeting .date {
    margin: 0 0 6px;
    font-size: 14px;
    color: var(--text-soft);
  }
  .greeting h1 {
    margin: 0;
    font-size: 32px;
    font-weight: 650;
    line-height: 1.2;
    letter-spacing: -0.015em;
  }
  .greeting .summary {
    margin: 12px 0 0;
    font-size: 17px;
    line-height: 1.6;
    color: var(--text);
    max-width: 34em;
  }

  section h2 {
    margin: 0 0 14px;
    font-size: 20px;
    font-weight: 650;
    line-height: 1.3;
    letter-spacing: -0.005em;
    color: var(--text);
  }

  /* Cards: no border, depth from shadow (light) or a raised surface (dark).
     The brand tint is a soft gradient; text keeps its normal colour and
     brandColor.ts lowers --brand-k where contrast would drop below 4.5:1. */
  .cards {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .card,
  .order {
    /* --tint / --tint-end come from .brand-scope (app.css): light fades out to
       the surface, dark keeps a clear colour at the end. */
    background: linear-gradient(160deg, var(--tint), var(--tint-end) 72%);
    box-shadow:
      var(--card-shadow),
      var(--card-highlight),
      0 14px 34px -16px color-mix(in srgb, var(--brand-acc) var(--glow-mix), transparent);
  }
  .card {
    position: relative;
    /* Dark theme: the left corners are tighter so the brand edge reads as a
       straight spine instead of bending around a large curve. */
    border-radius: var(--card-radius-edge) var(--card-radius) var(--card-radius) var(--card-radius-edge);
    /* The edge is a background layer, so it is clipped by the card's own
       corners (--edge-w is 0 in light). */
    background:
      linear-gradient(var(--brand-acc), var(--brand-acc)) left / var(--edge-w) 100% no-repeat,
      linear-gradient(160deg, var(--tint), var(--tint-end) 72%);
    padding: 20px;
    scroll-margin: 12px;
    /* Motion here never animates `transform`, `opacity` or `filter`. WebKitGTK
       runs those on the compositor and, when the animation ends, shows the
       start state again for one frame (a visible flicker, confirmed in a
       screen recording). `top` and `box-shadow` are painted on the main
       thread and end cleanly. */
    top: 0;
    transition: top 0.2s ease, box-shadow 0.2s ease;
  }
  .card.can-toggle:not(.expanded) {
    cursor: pointer;
  }
  .card.can-toggle:not(.expanded):hover,
  .card.can-toggle:not(.expanded):has(.card-body:focus-visible) {
    top: -2px;
  }
  .card.can-toggle:not(.expanded):hover,
  .card.can-toggle:not(.expanded):has(.card-body:focus-visible),
  .card.expanded {
    box-shadow:
      var(--card-shadow-hover),
      var(--card-highlight),
      0 18px 40px -14px color-mix(in srgb, var(--brand-acc) var(--glow-mix-hover), transparent);
  }
  .card.foryou .card-body {
    padding-right: 24px;
  }
  .card.expanded .card-body {
    padding-right: 64px;
  }
  .card-body.head-toggle {
    cursor: pointer;
    border-radius: var(--card-radius-sm);
  }
  .card-body.head-toggle:focus-visible {
    outline-offset: 6px;
  }
  .card-body {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    min-width: 0;
  }
  .texts {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .headline {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    line-height: 1.4;
  }
  .sub {
    margin: 0;
    font-size: 15px;
    line-height: 1.5;
    color: var(--text-soft);
  }
  .detail {
    margin: 0;
    font-size: 14px;
    line-height: 1.5;
    color: var(--text-soft);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* Logo tile: clean light tile with a soft shadow/ring and a faint brand glow */
  .avatar {
    flex: none;
    width: 52px;
    height: 52px;
    border-radius: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background: color-mix(in srgb, var(--brand-acc, var(--blue-slate)) 24%, var(--card-surface));
    /* light: flat 4px ring; dark: a soft glow in the brand colour */
    box-shadow:
      var(--tile-shadow),
      0 0 var(--halo-blur) var(--halo-spread) color-mix(in srgb, var(--brand-acc, transparent) var(--halo-mix), transparent);
    color: var(--text);
    font-size: 17px;
    font-weight: 650;
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

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: 16px 0 0 68px;
  }
  /* One filled primary per card; secondary = soft tinted pill; quiet = plain text */
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    border-radius: 99px;
    border: 0;
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.3;
    transition: background 0.16s ease;
  }
  .btn:hover {
    background: color-mix(in srgb, var(--text) calc(var(--pill-mix) + 6%), transparent);
  }
  .btn:active,
  .order-head:active,
  .undo-btn:active {
    transform: scale(0.98);
  }
  .btn.primary {
    /* light: logo colour; dark: the vivid accent with a pitch-black label */
    background: color-mix(in srgb, var(--brand-d, var(--accent-fill)) var(--dark-share), var(--brand-btn, var(--accent-fill)));
    color: color-mix(in srgb, var(--pitch-black) var(--dark-share), var(--brand-on, var(--on-accent-fill)));
  }
  .btn.primary:hover {
    filter: brightness(1.08);
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
  .btn.small {
    padding: 5px 12px;
    font-size: 13px;
  }

  /* All caught up */
  .caught-up {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 20px;
    border-radius: var(--card-radius);
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
  }
  .caught-up p {
    margin: 0;
    font-size: 16px;
    line-height: 1.55;
    color: var(--text);
  }
  .cu-icon {
    flex: none;
    width: 44px;
    height: 44px;
    border-radius: 14px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--highlight) 18%, var(--card-surface));
    color: var(--accent-text);
  }

  /* Orders strip: a "stories" row under the greeting */
  .strip-block {
    margin-bottom: 32px;
  }
  .strip-wrap {
    position: relative;
    margin: 0 -24px;
    --fade: 28px;
  }
  .strip-wrap.fade-left {
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 var(--fade));
    mask-image: linear-gradient(90deg, transparent, #000 var(--fade));
  }
  .strip-wrap.fade-right {
    -webkit-mask-image: linear-gradient(270deg, transparent, #000 var(--fade));
    mask-image: linear-gradient(270deg, transparent, #000 var(--fade));
  }
  .strip-wrap.fade-left.fade-right {
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 var(--fade), #000 calc(100% - var(--fade)), transparent);
    mask-image: linear-gradient(90deg, transparent, #000 var(--fade), #000 calc(100% - var(--fade)), transparent);
  }
  .strip {
    list-style: none;
    margin: 0;
    padding: 8px 24px 10px;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    overflow-y: hidden;
    overscroll-behavior-x: contain;
    scrollbar-width: none;
  }
  .strip::-webkit-scrollbar {
    display: none;
  }
  .strip li {
    flex: none;
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    width: 96px;
    padding: 8px 6px 8px;
    border: 0;
    border-radius: 18px;
    background: none;
    text-align: center;
  }
  /* Hover only lifts the logo; a background fading in at the same time made
     the hover look like two animations starting one after the other. */
  .tile.open {
    background: color-mix(in srgb, var(--text) 7%, transparent);
  }
  .tile.muted {
    opacity: 0.6;
  }
  .tile.muted:hover,
  .tile.muted.open,
  .tile.muted:focus-visible {
    opacity: 0.9;
  }
  .tile-art {
    position: relative;
    display: block;
    width: 64px;
    height: 64px;
    margin-bottom: 4px;
    /* keep the whole deck (front tile plus layers peeking out right) centred */
    transform: translateX(calc(var(--extra, 0) * -4.5px));
    top: 0;
    transition: top 0.18s ease;
  }
  .tile:hover .tile-art {
    top: -2px;
  }
  .layer {
    position: absolute;
    inset: 6px;
    border-radius: 50%;
    transform: translateX(calc(var(--d) * 9px)) scale(calc(1 - var(--d) * 0.06));
    background: color-mix(in srgb, var(--brand-acc) 34%, var(--card-surface));
    box-shadow: var(--tile-shadow);
    opacity: calc(1 - var(--d) * 0.18);
  }
  .ring {
    position: absolute;
    inset: 0;
    width: 64px;
    height: 64px;
    transform: rotate(-90deg);
    fill: none;
    stroke-width: 3;
    stroke-linecap: round;
  }
  .r-track {
    stroke: color-mix(in srgb, var(--text) 16%, transparent);
    stroke-dasharray: 20 5;
  }
  .r-fill {
    stroke: var(--brand-acc);
  }
  .tile .avatar {
    position: absolute;
    left: 6px;
    top: 6px;
    /* round, like the progress ring around it */
    border-radius: 50%;
    box-shadow: var(--tile-shadow);
  }
  .attn {
    position: absolute;
    right: 1px;
    top: 1px;
    width: 10px;
    height: 10px;
    border-radius: 99px;
    background: var(--ochre);
    box-shadow: 0 0 0 2px var(--bg);
  }
  .t-name,
  .t-status {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .t-name {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.3;
    color: var(--text);
  }
  .t-status {
    font-size: 11.5px;
    line-height: 1.3;
    color: var(--text-soft);
  }
  .panel {
    margin-top: 6px;
  }
  .panel-title {
    margin: 4px 0 10px;
    font-size: 15px;
    font-weight: 650;
    color: var(--text);
  }
  .orders {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .order {
    border-radius: var(--card-radius-sm);
  }
  .order.arrived {
    opacity: 0.7;
  }
  .order-head {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    grid-template-areas:
      'logo texts chip'
      'line line line';
    align-items: center;
    column-gap: 12px;
    row-gap: 14px;
    width: 100%;
    padding: 14px 14px 12px;
    border: 0;
    border-radius: var(--card-radius-sm);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .order-head.static {
    cursor: default;
  }
  .order-head .avatar {
    grid-area: logo;
    width: 40px;
    height: 40px;
    border-radius: 13px;
    font-size: 14px;
    box-shadow: var(--tile-shadow);
  }
  .o-texts {
    grid-area: texts;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .o-shop {
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
  }
  .o-title,
  .o-shop {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .o-title {
    font-size: 13px;
    color: var(--text-soft);
  }
  .chip {
    grid-area: chip;
    padding: 3px 10px;
    border-radius: 99px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    color: var(--text);
    background: color-mix(in srgb, var(--brand-acc) var(--chip-mix), transparent);
  }

  /* One continuous rounded bar with four step dots */
  .progress {
    grid-area: line;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .track {
    position: relative;
    display: block;
    height: 6px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) 12%, transparent);
    margin: 0 5px;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 99px;
    background: var(--brand-acc);
    transition: width 0.3s ease;
  }
  .dot {
    position: absolute;
    top: 50%;
    width: 10px;
    height: 10px;
    margin: -5px 0 0 -5px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) 22%, var(--card-surface));
    box-shadow: 0 0 0 2px var(--card-surface);
  }
  .dot.reached {
    background: var(--brand-acc);
  }
  .steps {
    /* Natural widths instead of four equal columns: "In Zustellung" would
       wrap in the narrow side column otherwise. */
    display: flex;
    justify-content: space-between;
    gap: 6px;
    white-space: nowrap;
    font-size: 13px;
    line-height: 1.3;
    color: var(--text-soft);
  }
  .step {
    text-align: center;
  }
  .step:first-child {
    text-align: left;
  }
  .step:last-child {
    text-align: right;
  }
  .step.current {
    font-weight: 650;
    color: var(--text);
  }
  .hint-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 0 14px 14px;
  }
  .hint {
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-soft);
    font-style: italic;
  }
  .order-detail {
    margin: 0 14px;
    padding: 14px 0 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    border-top: 1px solid color-mix(in srgb, var(--text) 10%, transparent);
  }
  .facts {
    margin: 0;
    display: grid;
    gap: 6px;
    font-size: 14px;
  }
  .facts div {
    display: flex;
    gap: 8px;
    justify-content: space-between;
  }
  .facts dt {
    color: var(--text-soft);
  }
  .facts dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
    text-align: right;
    overflow-wrap: anywhere;
  }
  .timeline,
  .attachments {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    font-size: 14px;
  }
  .timeline li {
    position: relative;
    display: flex;
    gap: 10px;
    padding: 0 0 10px 18px;
  }
  .timeline li:last-child {
    padding-bottom: 0;
  }
  .t-dot {
    position: absolute;
    left: 0;
    top: 7px;
    width: 8px;
    height: 8px;
    border-radius: 99px;
    background: var(--brand-acc);
  }
  /* connecting line between the dots */
  .timeline li:not(:last-child)::before {
    content: '';
    position: absolute;
    left: 3px;
    top: 17px;
    bottom: -7px;
    width: 2px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) 14%, transparent);
  }
  .t-date {
    flex: none;
    width: 52px;
    color: var(--text-soft);
    font-variant-numeric: tabular-nums;
  }
  .attachments {
    gap: 4px;
  }
  .attachments li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .muted {
    color: var(--text-soft);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    text-align: left;
    color: var(--accent-text);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .order-detail .actions {
    margin: 0;
  }

  /* Dismiss "×": quiet at rest, clear on hover or focus */
  .dismiss {
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
  .card-tools {
    position: absolute;
    top: 10px;
    right: 10px;
    display: flex;
    gap: 2px;
    z-index: 1;
  }
  .fs-icon {
    display: inline-flex;
    /* "forward" turned into a diagonal arrow: opens the large view */
    transform: rotate(-45deg);
  }
  .card:hover .dismiss,
  .card.expanded .dismiss,
  .quiet-list li:hover > .dismiss,
  .dismiss:focus-visible {
    opacity: 1;
  }
  .dismiss:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }

  /* Mail inside an expanded card */
  .mail {
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px solid color-mix(in srgb, var(--text) 12%, transparent);
    cursor: default;
  }
  .gone {
    margin: 0;
    padding: 4px 0 2px;
    font-size: 14px;
    color: var(--text-soft);
  }

  .undo-bar {
    position: absolute;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 8px 18px;
    border-radius: 99px;
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight), 0 0 0 1px color-mix(in srgb, var(--text) 8%, transparent);
    font-size: 14px;
    white-space: nowrap;
  }
  .undo-btn {
    padding: 5px 14px;
    border: 0;
    border-radius: 99px;
    background: none;
    font-size: 14px;
    font-weight: 600;
    color: var(--accent-text);
    cursor: pointer;
    transition: background 0.16s ease;
  }
  .undo-btn:hover {
    background: var(--hover);
  }

  /* Quiet list and "Wie immer": one soft card each, comfortable rows */
  .quiet-list,
  .usual-card {
    list-style: none;
    margin: 0;
    padding: 6px;
    border-radius: var(--card-radius-sm);
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
  }
  .quiet-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .quiet-list li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 52px;
    padding: 8px 10px;
    border-radius: 12px;
    transition: background 0.16s ease;
  }
  .quiet-list li:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .q-icon {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text-soft);
  }
  .q-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 0;
    background: none;
    padding: 2px 0;
    text-align: left;
    cursor: pointer;
  }
  .q-text:disabled {
    cursor: default;
  }
  .q-main {
    font-size: 15px;
    line-height: 1.45;
  }
  .q-sub {
    font-size: 13px;
    color: var(--text-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .usual-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 48px;
    padding: 10px 12px;
    border: 0;
    border-radius: 12px;
    background: none;
    text-align: left;
    font-size: 15px;
    line-height: 1.45;
    color: var(--text-soft);
    transition: background 0.16s ease, color 0.16s ease;
  }
  .usual-toggle:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
    color: var(--text);
  }
  .chev {
    display: inline-flex;
    flex: none;
  }
  .usual-list {
    list-style: none;
    margin: 2px 0 0;
    padding: 0;
  }
  .usual-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    min-height: 44px;
    padding: 10px 12px 10px 36px;
    border: 0;
    border-radius: 12px;
    background: none;
    text-align: left;
    font-size: 14px;
    min-width: 0;
    transition: background 0.16s ease;
  }
  .usual-row:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .u-sender {
    flex: none;
    max-width: 40%;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .u-dot {
    color: var(--text-soft);
  }
  .u-subject {
    flex: 1;
    min-width: 0;
    color: var(--text-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .card,
    .tile,
    .tile-art,
    .btn,
    .order-head,
    .undo-btn,
    .dismiss,
    .fill,
    .quiet-list li,
    .usual-toggle,
    .usual-row {
      transition: none;
    }
    .card.can-toggle:not(.expanded):hover,
    .card.can-toggle:not(.expanded):has(.card-body:focus-visible),
    .tile:hover .tile-art {
      top: 0;
    }
  }

  @media (max-width: 760px) {
    .today {
      padding: 28px 16px 64px;
    }
    .greeting h1 {
      font-size: 28px;
    }
    .actions {
      margin-left: 0;
    }
    .strip-wrap {
      margin: 0 -16px;
    }
    .strip {
      padding: 8px 16px 10px;
    }
  }
</style>

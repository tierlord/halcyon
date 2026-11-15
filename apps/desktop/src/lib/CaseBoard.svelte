<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import { caseKindLabel, dayOffset, type AsideCategory, type CaseItem, type CaseStatus } from './cases'
  import type { SourceMode, SourceRecord } from './types'

  let {
    cases,
    categories,
    sources = [],
    selectedCaseId,
    hoveredCaseId,
    onselectcase,
    onhovercase,
    onopenmessage,
    onchangestatus,
    onchangesourcemode,
    ondismissanomaly,
    onremovesource,
  }: {
    cases: CaseItem[]
    categories: AsideCategory[]
    sources?: SourceRecord[]
    selectedCaseId: number | null
    hoveredCaseId: number | null
    onselectcase: (id: number | null) => void
    onhovercase: (id: number | null) => void
    onopenmessage: (messageId: number) => void
    onchangestatus: (caseId: number, status: CaseStatus) => void
    onchangesourcemode?: (sourceId: number, mode: SourceMode) => void
    ondismissanomaly?: (sourceId: number) => void
    onremovesource?: (sourceId: number, asJunk: boolean) => void
  } = $props()

  let logoVersion = $state(0)
  let activeTab = $state<'tasks' | 'orders' | 'sources'>('tasks')
  let showDone = $state(false)
  let orderSearch = $state('')
  let orderYear = $state<string>('all')
  let openCategoryId = $state<string | null>(null)
  let sourceSearch = $state('')
  let sourceSort = $state<'recent' | 'count'>('recent')

  const today = dayOffset(0)
  const yesterday = dayOffset(-1)

  onMount(() => {
    return subscribeLogos(() => {
      logoVersion += 1
    })
  })

  const nonOrderCases = $derived(cases.filter((c) => c.kind !== 'order'))
  const orderCases = $derived(cases.filter((c) => c.kind === 'order'))

  const needsYou = $derived(nonOrderCases.filter((c) => c.status === 'needs_you'))
  const waiting = $derived(nonOrderCases.filter((c) => c.status === 'waiting'))
  const doneCases = $derived(nonOrderCases.filter((c) => c.status === 'done'))
  const anomalies = $derived(sources.filter((s) => s.anomaly != null))
  const recurringSources = $derived(sources.filter((s) => s.total_messages >= 2))
  const filteredSources = $derived.by(() => {
    const q = sourceSearch.trim().toLowerCase()
    const list = recurringSources.filter((s) => {
      if (!q) return true
      if (s.display_name.toLowerCase().includes(q)) return true
      if (s.source_key.toLowerCase().includes(q)) return true
      if (s.domain && s.domain.toLowerCase().includes(q)) return true
      if (s.patterns[0] && s.patterns[0].toLowerCase().includes(q)) return true
      return false
    })
    return list.slice().sort((a, b) =>
      sourceSort === 'recent'
        ? b.last_seen_at - a.last_seen_at
        : b.total_messages - a.total_messages || b.last_seen_at - a.last_seen_at,
    )
  })

  const activeOrders = $derived(
    orderCases
      .filter((c) => c.status !== 'done')
      .sort((a, b) => (b.orderDate ?? '').localeCompare(a.orderDate ?? '')),
  )
  const archivedOrdersAll = $derived(
    orderCases
      .filter((c) => c.status === 'done')
      .sort((a, b) => (b.orderDate ?? '').localeCompare(a.orderDate ?? '')),
  )
  const availableOrderYears = $derived.by(() => {
    const years = new Set<string>()
    for (const c of archivedOrdersAll) {
      if (c.orderDate && c.orderDate.length >= 4) {
        years.add(c.orderDate.slice(0, 4))
      }
    }
    return Array.from(years).sort((a, b) => b.localeCompare(a))
  })
  const filteredArchivedOrders = $derived.by(() => {
    const q = orderSearch.trim().toLowerCase()
    return archivedOrdersAll.filter((c) => {
      if (orderYear !== 'all' && (!c.orderDate || !c.orderDate.startsWith(orderYear))) {
        return false
      }
      if (!q) return true
      if (c.title.toLowerCase().includes(q) || c.counterparty.toLowerCase().includes(q)) {
        return true
      }
      return c.keys.some((k) => k.value.toLowerCase().includes(q))
    })
  })
  const archivedOrdersTotalEur = $derived.by(() => {
    let cents = 0
    let countWithAmount = 0
    for (const c of filteredArchivedOrders) {
      if (c.nextStep === 'Storniert') continue
      if (c.amountCents != null && (c.currency ?? 'EUR') === 'EUR') {
        cents += c.amountCents
        countWithAmount += 1
      }
    }
    if (countWithAmount === 0) return null
    return new Intl.NumberFormat('de-DE', { style: 'currency', currency: 'EUR' }).format(
      cents / 100,
    )
  })

  const tasksBadge = $derived(needsYou.length + anomalies.length + waiting.length)
  const ordersBadge = $derived(
    activeOrders.length > 0
      ? `${activeOrders.length} offen · ${archivedOrdersAll.length}`
      : `${archivedOrdersAll.length}`,
  )
  const sourcesBadge = $derived(recurringSources.length + categories.length)

  const summary = $derived.by(() => {
    const urgentCount = needsYou.length + anomalies.length
    const openCount = waiting.length
    const openOrders = activeOrders.length
    const parts: string[] = []
    if (urgentCount > 0) {
      parts.push(
        urgentCount === 1 ? '1 Aufgabe braucht dich' : `${urgentCount} Aufgaben brauchen dich`,
      )
    }
    if (openCount > 0) {
      parts.push(openCount === 1 ? '1 Vorgang wartet' : `${openCount} Vorgänge warten`)
    }
    if (openOrders > 0) {
      parts.push(
        openOrders === 1 ? '1 Bestellung unterwegs' : `${openOrders} Bestellungen unterwegs`,
      )
    }
    return parts.length > 0 ? parts.join(' · ') : 'Alles auf dem Laufenden'
  })

  function formatEventDay(dayIso: string): string {
    if (dayIso === today) return 'Heute'
    if (dayIso === yesterday) return 'Gestern'
    return `${dayIso.slice(8, 10)}.${dayIso.slice(5, 7)}.`
  }

  function formatArchiveDate(dayIso?: string): string {
    if (!dayIso || dayIso.length < 10) return '—'
    return `${dayIso.slice(8, 10)}.${dayIso.slice(5, 7)}.${dayIso.slice(0, 4)}`
  }

  function formatUnixDate(ts: number | null | undefined): string {
    if (ts == null || ts <= 0) return '—'
    const d = new Date(ts * 1000)
    const now = new Date()
    if (
      d.getDate() === now.getDate() &&
      d.getMonth() === now.getMonth() &&
      d.getFullYear() === now.getFullYear()
    ) {
      return 'Heute'
    }
    const yesterdayDate = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
    if (
      d.getDate() === yesterdayDate.getDate() &&
      d.getMonth() === yesterdayDate.getMonth() &&
      d.getFullYear() === yesterdayDate.getFullYear()
    ) {
      return 'Gestern'
    }
    const day = String(d.getDate()).padStart(2, '0')
    const month = String(d.getMonth() + 1).padStart(2, '0')
    const year = d.getFullYear()
    return `${day}.${month}.${year}`
  }

  function primaryOrderNumber(c: CaseItem): string | null {
    const found = c.keys.find(
      (k) => k.label === 'Bestellnr.' || k.label === 'Nummer' || k.label === 'Rechnungsnr.',
    )
    return found ? found.value : null
  }

  function primaryOrderMessageId(c: CaseItem): number | null {
    for (const ev of c.events) {
      if (ev.messageId != null) return ev.messageId
    }
    return c.messageIds[0] ?? null
  }

  function logoFor(domain: string | null) {
    void logoVersion
    return domain ? getBrandLogo(`info@${domain}`) : null
  }
</script>

{#snippet caseCard(c: CaseItem, urgent: boolean)}
  {@const expanded = selectedCaseId === c.id}
  {@const logo = logoFor(c.domain)}
  <article
    class="card"
    class:needs-you={urgent}
    class:expanded
    class:hovered={hoveredCaseId === c.id}
    onmouseenter={() => onhovercase(c.id)}
    onmouseleave={() => onhovercase(null)}
  >
    {#if logo?.bg}
      <span class="card-wash" style:--brand-wash={logo.bg} aria-hidden="true"></span>
    {/if}
    <button
      type="button"
      class="card-main"
      onclick={() => onselectcase(expanded ? null : c.id)}
      aria-expanded={expanded}
    >
      <div class="card-top">
        {#if logo}
          <span class="card-logo" aria-hidden="true">
            <img src={logo.dataUrl} alt="" class:opaque={!logo.padded} />
          </span>
        {/if}
        <div class="card-titles">
          <div class="card-row1">
            <span class="card-title">{c.title}</span>
            <span class="kind-chip">{caseKindLabel(c.kind)}</span>
          </div>
          <span class="card-counterparty">{c.counterparty}</span>
        </div>
        {#if c.amount && c.dueLabel}
          <span class="due-badge" class:urgent>{c.amount} · {c.dueLabel}</span>
        {:else if c.amount || c.dueLabel}
          <span class="due-badge" class:urgent>{c.amount ?? c.dueLabel}</span>
        {/if}
      </div>
      <p class="next-step">{c.nextStep}</p>
      <div class="mini-steps" aria-hidden="true">
        {#each c.events as ev, i}
          <span class="step-pill" class:due={ev.phase === 'due'}>
            {ev.label}
          </span>
          {#if i < c.events.length - 1}<span class="step-arrow">→</span>{/if}
        {/each}
      </div>
    </button>

    {#if expanded}
      <div class="card-details">
        <div class="keys">
          {#each c.keys as k}
            <span class="key-chip"><strong>{k.label}:</strong> {k.value}</span>
          {/each}
        </div>
        <div class="detail-events" aria-label="Verlauf">
          {#each c.events as ev (ev.id)}
            <div class="detail-ev">
              <span class="ev-marker" class:due={ev.phase === 'due'} class:done={ev.phase === 'done'} aria-hidden="true"></span>
              <span class="ev-date" class:is-today={ev.day === today}>{formatEventDay(ev.day)}</span>
              <span class="ev-text">{ev.label}</span>
              {#if ev.messageId != null}
                <button
                  type="button"
                  class="ev-open"
                  onclick={() => onopenmessage(ev.messageId!)}
                >
                  Mail öffnen
                </button>
              {/if}
            </div>
          {/each}
        </div>
        <p class="reason"><strong>Warum?</strong> {c.reason}</p>
        <div class="card-actions">
          <button type="button" class="act-btn primary" onclick={() => onchangestatus(c.id, 'done')}>
            <Icon name="check" size={14} /> Erledigt
          </button>
          <button
            type="button"
            class="act-btn"
            title="Falsch erkannt: Karte entfernen, ohne Mails zu löschen"
            onclick={() => onchangestatus(c.id, 'rejected')}
          >
            Kein Vorgang
          </button>
          <button type="button" class="act-btn" onclick={() => onselectcase(null)}>
            Schließen
          </button>
        </div>
      </div>
    {/if}

    {#if c.attachments && c.attachments.length > 0}
      <div class="card-attachments" aria-label="Angehängte Dokumente">
        <Icon name="paperclip" size={12} />
        <div class="att-list">
          {#each c.attachments as att (att.id)}
            <button
              type="button"
              class="att-chip"
              onclick={() =>
                att.messageId != null
                  ? onopenmessage(att.messageId)
                  : onselectcase(expanded ? null : c.id)}
              title={att.messageId != null ? 'Zugehörige Mail öffnen' : att.name}
            >
              <span class="att-name">{att.name}</span>
              {#if att.sizeLabel}
                <span class="att-size">{att.sizeLabel}</span>
              {/if}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </article>
{/snippet}

<section class="board" aria-label="Vorgänge">
  <header class="board-head">
    <div class="board-head-top">
      <div>
        <h1>Vorgänge</h1>
        <p class="subtitle">
          {summary}
        </p>
      </div>
      <nav class="board-tabs" aria-label="Bereiche in Vorgänge">
        <button
          type="button"
          class="board-tab"
          class:active={activeTab === 'tasks'}
          onclick={() => (activeTab = 'tasks')}
        >
          <span>Aufgaben & Verträge</span>
          <span class="tab-badge" class:urgent={needsYou.length + anomalies.length > 0}>
            {tasksBadge}
          </span>
        </button>
        <button
          type="button"
          class="board-tab"
          class:active={activeTab === 'orders'}
          onclick={() => (activeTab = 'orders')}
        >
          <span>Bestellungen & Einkäufe</span>
          <span class="tab-badge">{ordersBadge}</span>
        </button>
        <button
          type="button"
          class="board-tab"
          class:active={activeTab === 'sources'}
          onclick={() => (activeTab = 'sources')}
        >
          <span>Wiederkehrende Infos</span>
          <span class="tab-badge">{sourcesBadge}</span>
        </button>
      </nav>
    </div>
  </header>

  <div class="board-scroll">
    {#if activeTab === 'tasks'}
      {#if needsYou.length === 0 && anomalies.length === 0 && waiting.length === 0 && doneCases.length === 0}
        <div class="empty">
          <p class="empty-title">Keine offenen Aufgaben oder Verträge</p>
          <p class="empty-text">
            Offene Rechnungen zum Überweisen, Vertragsänderungen, Kündigungen und ungewöhnliche
            Abweichungen erscheinen hier.
          </p>
        </div>
      {/if}

      {#if needsYou.length > 0 || anomalies.length > 0}
        <section class="group">
          <h2 class="group-title urgent">Braucht dich</h2>
          <div class="cards">
            {#each anomalies as s (`anom-${s.id}`)}
              <article class="card anomaly-card" class:needs-you={s.anomaly?.needs_you}>
                <div class="card-main">
                  <div class="card-top">
                    <Icon name="warning" size={16} />
                    <span class="card-title">{s.display_name}</span>
                    <span class="kind-chip">Anders als sonst</span>
                    <span class="due-badge" class:urgent={s.anomaly?.needs_you}>
                      {s.anomaly?.needs_you ? 'Prüfen' : 'Hinweis'}
                    </span>
                  </div>
                  <p class="next-step">{s.anomaly?.reason}</p>
                  <p class="reason">
                    <strong>Warum?</strong> Quelle <code>{s.source_key}</code> ({s.total_messages} Mails{s.median_interval_secs
                      ? `, üblicher Abstand ~${Math.max(1, Math.round(s.median_interval_secs / 86400))} Tage`
                      : ''})
                  </p>
                  <div class="card-actions">
                    {#if s.latest_message_id != null}
                      <button type="button" class="act-btn" onclick={() => onopenmessage(s.latest_message_id!)}>Mail öffnen</button>
                    {/if}
                    {#if ondismissanomaly}
                      <button type="button" class="act-btn" onclick={() => ondismissanomaly(s.id)}>Verstanden</button>
                    {/if}
                  </div>
                </div>
              </article>
            {/each}
            {#each needsYou as c (c.id)}
              {@render caseCard(c, true)}
            {/each}
          </div>
        </section>
      {/if}

      {#if waiting.length > 0}
        <section class="group">
          <h2 class="group-title">Wartet auf Rückmeldung</h2>
          <div class="cards">
            {#each waiting as c (c.id)}
              {@render caseCard(c, false)}
            {/each}
          </div>
        </section>
      {/if}

      {#if doneCases.length > 0}
        <section class="group done-group">
          <button type="button" class="collapse-btn" onclick={() => (showDone = !showDone)}>
            <Icon name={showDone ? 'chevronDown' : 'chevronRight'} size={14} />
            <span>Erledigte Vorgänge ({doneCases.length})</span>
          </button>
          {#if showDone}
            <div class="cards muted-cards">
              {#each doneCases as c (c.id)}
                {@render caseCard(c, false)}
              {/each}
            </div>
          {/if}
        </section>
      {/if}
    {:else if activeTab === 'orders'}
      {#if activeOrders.length === 0 && archivedOrdersAll.length === 0}
        <div class="empty">
          <p class="empty-title">Noch keine Bestellungen erkannt</p>
          <p class="empty-text">
            Bestell-, Versand- und Lieferbestätigungen werden hier automatisch mit Betrag und
            Bestellnummer gebündelt.
          </p>
        </div>
      {/if}

      {#if activeOrders.length > 0}
        <section class="group">
          <h2 class="group-title">Unterwegs ({activeOrders.length})</h2>
          <div class="cards">
            {#each activeOrders as c (c.id)}
              {@render caseCard(c, c.status === 'needs_you')}
            {/each}
          </div>
        </section>
      {/if}

      {#if archivedOrdersAll.length > 0}
        <section class="group">
          <div class="archive-header">
            <h2 class="group-title archive-title">
              Bestell-Archiv ({filteredArchivedOrders.length}{filteredArchivedOrders.length !== archivedOrdersAll.length ? ` von ${archivedOrdersAll.length}` : ''})
            </h2>
            {#if archivedOrdersTotalEur}
              <span class="archive-sum" title="Summe der angezeigten Bestellungen (ohne Stornierungen)">
                Summe: <strong>{archivedOrdersTotalEur}</strong>
              </span>
            {/if}
          </div>

          <div class="archive-toolbar">
            <input
              type="search"
              class="archive-search"
              placeholder="Artikel, Shop oder Bestellnr. suchen…"
              bind:value={orderSearch}
            />
            {#if availableOrderYears.length > 1}
              <div class="mode-switch">
                <button
                  type="button"
                  class="mode-btn"
                  class:active={orderYear === 'all'}
                  onclick={() => (orderYear = 'all')}
                >
                  Alle
                </button>
                {#each availableOrderYears as yr (yr)}
                  <button
                    type="button"
                    class="mode-btn"
                    class:active={orderYear === yr}
                    onclick={() => (orderYear = yr)}
                  >
                    {yr}
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <div class="order-archive-list" role="list">
            {#each filteredArchivedOrders as c (c.id)}
              {@const expanded = selectedCaseId === c.id}
              {@const logo = logoFor(c.domain)}
              {@const orderNo = primaryOrderNumber(c)}
              {@const msgId = primaryOrderMessageId(c)}
              {@const isCancelled = c.nextStep === 'Storniert'}
              <article
                class="order-row"
                class:expanded
                class:cancelled={isCancelled}
                onmouseenter={() => onhovercase(c.id)}
                onmouseleave={() => onhovercase(null)}
              >
                <div class="order-row-line">
                  <button
                    type="button"
                    class="order-row-main"
                    onclick={() => onselectcase(expanded ? null : c.id)}
                    aria-expanded={expanded}
                  >
                    <span class="order-date">{formatArchiveDate(c.orderDate)}</span>
                    <span class="order-shop">
                      {#if logo}
                        <img src={logo.dataUrl} alt="" class="order-shop-logo" />
                      {/if}
                      <span class="order-shop-name">{c.counterparty}</span>
                    </span>
                    <span class="order-title-col">
                      <span class="order-title" title={c.title}>{c.title}</span>
                      {#if orderNo}
                        <span class="order-no-chip">{orderNo}</span>
                      {/if}
                    </span>
                    <span class="order-status-chip" class:cancelled={isCancelled}>
                      {c.nextStep === 'Ohne Neuigkeiten abgeschlossen' ? 'Abgeschlossen' : c.nextStep}
                    </span>
                    <span class="order-amount" class:cancelled={isCancelled}>
                      {c.amount ?? '—'}
                    </span>
                  </button>
                  {#if msgId != null}
                    <button
                      type="button"
                      class="ev-open order-open-btn"
                      onclick={() => onopenmessage(msgId)}
                      title="Bestell- oder Rechnungs-Mail öffnen"
                    >
                      Beleg / Mail
                    </button>
                  {/if}
                </div>

                {#if expanded}
                  <div class="card-details">
                    {#if c.keys.length > 0}
                      <div class="keys">
                        {#each c.keys as k}
                          <span class="key-chip"><strong>{k.label}:</strong> {k.value}</span>
                        {/each}
                      </div>
                    {/if}
                    <div class="detail-events" aria-label="Verlauf">
                      {#each c.events as ev (ev.id)}
                        <div class="detail-ev">
                          <span class="ev-marker" class:due={ev.phase === 'due'} class:done={ev.phase === 'done'} aria-hidden="true"></span>
                          <span class="ev-date" class:is-today={ev.day === today}>{formatArchiveDate(ev.day)}</span>
                          <span class="ev-text">{ev.label}</span>
                          {#if ev.messageId != null}
                            <button
                              type="button"
                              class="ev-open"
                              onclick={() => onopenmessage(ev.messageId!)}
                            >
                              Mail öffnen
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                    <p class="reason"><strong>Warum?</strong> {c.reason}</p>
                    <div class="card-actions">
                      <button
                        type="button"
                        class="act-btn"
                        title="Falsch erkannt: Karte entfernen, ohne Mails zu löschen"
                        onclick={() => onchangestatus(c.id, 'rejected')}
                      >
                        Kein Vorgang
                      </button>
                      <button type="button" class="act-btn" onclick={() => onselectcase(null)}>
                        Schließen
                      </button>
                    </div>
                  </div>
                {/if}
              </article>
            {/each}
          </div>
        </section>
      {/if}
    {:else if activeTab === 'sources'}
      {#if categories.length === 0 && recurringSources.length === 0}
        <div class="empty">
          <p class="empty-title">Keine wiederkehrenden Info-Quellen</p>
          <p class="empty-text">
            Regelmäßige Info-Mails, Newsletter und Status-Abos erscheinen hier und lassen sich
            pro Absender bündeln oder stummschalten.
          </p>
        </div>
      {/if}

      {#if categories.length > 0}
        <section class="group aside-group">
          <h2 class="group-title muted">Wiederkehrende Hinweise</h2>
          <div class="aside-chips">
            {#each categories as cat (cat.id)}
              <div class="aside-chip-wrap">
                <button
                  type="button"
                  class="aside-chip"
                  class:open={openCategoryId === cat.id}
                  onclick={() => (openCategoryId = openCategoryId === cat.id ? null : cat.id)}
                >
                  <Icon name={cat.icon} size={14} />
                  <span>{cat.label}</span>
                  <span class="aside-count">{cat.count}</span>
                </button>
                {#if openCategoryId === cat.id}
                  <div class="aside-detail">
                    <span>{cat.summary}</span>
                    {#if cat.messageIds[0] != null}
                      <button type="button" class="ev-open" onclick={() => onopenmessage(cat.messageIds[0])}>
                        Neueste öffnen
                      </button>
                    {/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        </section>
      {/if}

      {#if recurringSources.length > 0}
        <section class="group sources-group">
          <h2 class="group-title muted">
            Wiederkehrende Quellen ({filteredSources.length}{filteredSources.length !== recurringSources.length ? ` von ${recurringSources.length}` : ''})
          </h2>
          <div class="archive-toolbar">
            <input
              type="search"
              class="archive-search"
              placeholder="Quelle oder Betreffmuster suchen…"
              bind:value={sourceSearch}
            />
            <div class="mode-switch">
              <button
                type="button"
                class="mode-btn"
                class:active={sourceSort === 'recent'}
                onclick={() => (sourceSort = 'recent')}
              >
                Neueste zuerst
              </button>
              <button
                type="button"
                class="mode-btn"
                class:active={sourceSort === 'count'}
                onclick={() => (sourceSort = 'count')}
              >
                Häufigste zuerst
              </button>
            </div>
          </div>
          <div class="source-list">
            {#if filteredSources.length === 0}
              <div class="empty" style="margin: 24px auto;">
                <p class="empty-title">Keine Quellen gefunden</p>
                <p class="empty-text">Keine Quelle entspricht der aktuellen Suche.</p>
              </div>
            {/if}
            {#each filteredSources as s (s.id)}
              <div class="source-row">
                <div class="source-top">
                  <div class="source-ident">
                    <span class="source-name" title={s.source_key}>{s.display_name}</span>
                    <span class="kind-chip">{s.total_messages}×</span>
                    <span class="source-domain">{s.domain ?? s.source_key}</span>
                    <span class="source-date">Zuletzt: {formatUnixDate(s.last_seen_at)}</span>
                  </div>
                  {#if s.latest_message_id != null}
                    <button type="button" class="ev-open" onclick={() => onopenmessage(s.latest_message_id!)}>
                      Neueste öffnen
                    </button>
                  {/if}
                </div>
                <div class="source-bottom">
                  <span class="source-pattern" title={s.patterns[0] ?? s.source_key}>
                    {s.patterns[0] ?? s.source_key}
                  </span>
                  <div class="source-actions">
                    <div class="mode-switch">
                      {#each [
                        ['show', 'Einzeln'],
                        ['bundle', 'Bündeln'],
                        ['latest_only', 'Nur neueste'],
                        ['mute', 'Stumm']
                      ] as [modeVal, modeLabel]}
                        <button
                          type="button"
                          class="mode-btn"
                          class:active={s.mode === modeVal}
                          onclick={() => onchangesourcemode?.(s.id, modeVal as SourceMode)}
                        >
                          {modeLabel}
                        </button>
                      {/each}
                    </div>
                    <button
                      type="button"
                      class="source-action-btn"
                      title="Alle Mails dieser Quelle als Spam verschieben und Quelle entfernen"
                      onclick={() => onremovesource?.(s.id, true)}
                    >
                      Spam
                    </button>
                    <button
                      type="button"
                      class="source-action-btn"
                      title="Alle Mails dieser Quelle in den Papierkorb verschieben"
                      onclick={() => onremovesource?.(s.id, false)}
                    >
                      Löschen
                    </button>
                  </div>
                </div>
              </div>
            {/each}
          </div>
        </section>
      {/if}
    {/if}
  </div>
</section>

<style>
  .empty {
    margin: 48px auto;
    max-width: 360px;
    text-align: center;
    color: var(--text-muted);
  }
  .empty-title {
    font-weight: 600;
    color: var(--text);
    margin: 0 0 6px;
  }
  .empty-text {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .board {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--bg);
    overflow: hidden;
  }
  .board-head {
    padding: 16px 20px 12px;
    border-bottom: 1px solid var(--border);
  }
  .board-head-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .board-tabs {
    display: inline-flex;
    align-items: center;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 3px;
    gap: 3px;
  }
  .board-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: none;
    padding: 5px 11px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    border-radius: calc(var(--radius) - 2px);
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .board-tab:hover {
    color: var(--text);
  }
  .board-tab.active {
    background: var(--surface-raised);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  }
  .tab-badge {
    font-size: 11px;
    font-weight: 650;
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .board-tab.active .tab-badge {
    background: var(--bg);
    color: var(--text);
  }
  .tab-badge.urgent {
    background: color-mix(in srgb, var(--ochre) 18%, var(--surface));
    border-color: color-mix(in srgb, var(--ochre) 45%, var(--border));
    color: var(--ochre);
  }
  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
  }
  .subtitle {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .board-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 14px 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .group-title {
    margin: 0 0 8px;
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .group-title.urgent {
    color: var(--ochre);
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card {
    position: relative;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-raised);
    transition: border-color 0.12s ease, box-shadow 0.12s ease;
  }
  .card.needs-you {
    box-shadow: inset 3px 0 0 var(--ochre);
  }
  .card.hovered,
  .card:hover {
    border-color: var(--highlight);
  }
  .card.expanded {
    border-color: var(--highlight);
    box-shadow: inset 3px 0 0 var(--highlight);
  }
  .card-wash {
    --wash-edge: color-mix(in srgb, var(--brand-wash) var(--wash-mix), var(--bg));
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 0;
    background: linear-gradient(
      90deg,
      var(--wash-edge) 0%,
      color-mix(in srgb, var(--wash-edge) 42%, transparent) 58px,
      transparent 140px
    );
  }
  .card-main {
    position: relative;
    z-index: 1;
    width: 100%;
    border: 0;
    background: none;
    padding: 11px 14px;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .card-logo {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .card-logo img {
    width: 22px;
    height: 22px;
    object-fit: contain;
  }
  .card-logo img.opaque {
    width: 26px;
    height: 26px;
    object-fit: cover;
    border-radius: 6px;
  }
  .card-titles {
    flex: 1;
    min-width: 0;
  }
  .card-row1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .card-title {
    font-weight: 650;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind-chip {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 99px;
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .card-counterparty {
    display: block;
    font-size: 12px;
    color: var(--text-muted);
  }
  .due-badge {
    font-size: 11px;
    font-weight: 650;
    padding: 2px 8px;
    border-radius: 99px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .due-badge.urgent {
    background: color-mix(in srgb, var(--ochre) 18%, transparent);
    border-color: var(--ochre);
    color: var(--text);
  }
  .next-step {
    margin: 0;
    font-size: 13px;
    color: var(--text);
  }
  .mini-steps {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .step-pill {
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--surface);
  }
  .step-pill.due {
    color: var(--ochre);
    font-weight: 600;
  }
  .card-details {
    position: relative;
    z-index: 1;
    padding: 10px 14px 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .keys {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .key-chip {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .detail-events {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-left: 2px;
  }
  .detail-events::before {
    content: '';
    position: absolute;
    left: 5px;
    top: 7px;
    bottom: 7px;
    width: 1px;
    background: var(--border);
  }
  .detail-ev {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .ev-marker {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent-fill);
    flex-shrink: 0;
    z-index: 1;
  }
  .ev-marker.due {
    background: var(--ochre);
  }
  .ev-marker.done {
    background: var(--text-muted);
  }
  .ev-date {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    min-width: 48px;
  }
  .ev-date.is-today {
    color: var(--accent-text);
    font-weight: 650;
  }
  .ev-text {
    flex: 1;
  }
  .ev-open {
    border: 0;
    background: none;
    padding: 0;
    font-size: 12px;
    color: var(--accent-text);
    font-weight: 600;
  }
  .reason {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .act-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius);
    padding: 5px 10px;
    font-size: 12px;
    font-weight: 600;
  }
  .act-btn:hover {
    background: var(--hover);
  }
  .act-btn.primary {
    background: var(--accent-fill);
    border-color: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .card-attachments {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 65%, transparent);
    color: var(--text-muted);
  }
  .att-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .att-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    border-radius: 6px;
    padding: 2px 8px;
    font-size: 11px;
    color: var(--text);
  }
  .att-chip:hover {
    border-color: var(--highlight);
    color: var(--accent-text);
  }
  .att-size {
    color: var(--text-muted);
    font-size: 10px;
  }
  .aside-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .aside-chip-wrap {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .aside-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: var(--surface);
    font-size: 12px;
    color: var(--text-muted);
  }
  .aside-chip:hover,
  .aside-chip.open {
    color: var(--text);
    border-color: var(--highlight);
  }
  .aside-count {
    font-weight: 650;
    font-size: 11px;
  }
  .aside-detail {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    border-radius: var(--radius);
    background: var(--surface);
    font-size: 12px;
    color: var(--text-muted);
  }
  .collapse-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: none;
    padding: 2px 0;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
  }
  .anomaly-card {
    border-color: var(--ochre);
  }
  .source-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
  .source-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 14px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .source-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }
  .source-ident {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }
  .source-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .source-domain {
    font-size: 11px;
    color: var(--text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .source-date {
    font-size: 11px;
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .source-bottom {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }
  .source-pattern {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
  .source-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .source-action-btn {
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-muted);
    padding: 3px 8px;
    font-size: 11px;
    font-weight: 600;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all 0.12s ease;
    line-height: 1.4;
  }
  .source-action-btn:hover {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
    background: color-mix(in srgb, var(--danger) 8%, var(--surface));
  }
  .mode-switch {
    display: inline-flex;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 2px;
    gap: 2px;
  }
  .mode-btn {
    border: 0;
    background: none;
    padding: 3px 8px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    border-radius: calc(var(--radius) - 2px);
    cursor: pointer;
  }
  .mode-btn:hover {
    color: var(--text);
  }
  .mode-btn.active {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .archive-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
  }
  .archive-title {
    margin: 0;
  }
  .archive-sum {
    font-size: 12px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .archive-sum strong {
    color: var(--text);
    font-weight: 650;
  }
  .archive-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 10px;
  }
  .archive-search {
    flex: 1;
    min-width: 160px;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .archive-search:focus {
    outline: 1px solid var(--highlight);
    border-color: var(--highlight);
  }
  .order-archive-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .order-row {
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    transition: border-color 0.12s ease;
  }
  .order-row:hover,
  .order-row.expanded {
    border-color: var(--highlight);
  }
  .order-row.cancelled {
    opacity: 0.72;
  }
  .order-row-line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 10px;
  }
  .order-row-main {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    padding: 8px 12px;
    display: grid;
    grid-template-columns: 82px minmax(105px, 145px) minmax(0, 1fr) auto 86px;
    align-items: center;
    gap: 10px;
    text-align: left;
    cursor: pointer;
    color: inherit;
  }
  .order-date {
    font-size: 12px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .order-shop {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .order-shop-logo {
    width: 16px;
    height: 16px;
    object-fit: contain;
    border-radius: 3px;
    flex-shrink: 0;
  }
  .order-shop-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .order-title-col {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .order-title {
    font-size: 13px;
    font-weight: 550;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .order-no-chip {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .order-status-chip {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: var(--radius-pill);
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .order-status-chip.cancelled {
    color: var(--text-subtle);
    border-style: dashed;
  }
  .order-amount {
    font-size: 13px;
    font-weight: 650;
    color: var(--text);
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .order-amount.cancelled {
    text-decoration: line-through;
    color: var(--text-subtle);
    font-weight: 500;
  }
  .order-open-btn {
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>

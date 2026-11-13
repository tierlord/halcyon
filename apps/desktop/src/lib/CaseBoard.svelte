<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import { caseKindLabel, dayOffset, type AsideCategory, type CaseItem, type CaseStatus } from './cases'

  let {
    cases,
    categories,
    selectedCaseId,
    hoveredCaseId,
    onselectcase,
    onhovercase,
    onopenmessage,
    onchangestatus,
  }: {
    cases: CaseItem[]
    categories: AsideCategory[]
    selectedCaseId: string | null
    hoveredCaseId: string | null
    onselectcase: (id: string | null) => void
    onhovercase: (id: string | null) => void
    onopenmessage: (messageId: number) => void
    onchangestatus: (caseId: string, status: CaseStatus) => void
  } = $props()

  let logoVersion = $state(0)
  let selectedDay = $state<string | null>(null)
  let showDone = $state(false)
  let openCategoryId = $state<string | null>(null)
  let timelineEl: HTMLDivElement | undefined = $state()

  onMount(() => {
    const unsub = subscribeLogos(() => {
      logoVersion += 1
    })
    // Scroll horizontal timeline slightly towards "Heute".
    if (timelineEl) {
      timelineEl.scrollLeft = Math.max(0, timelineEl.scrollWidth - timelineEl.clientWidth - 120)
    }
    return unsub
  })

  const today = dayOffset(0)
  const calendarDays = Array.from({ length: 14 }, (_, idx) => {
    const offset = idx - 9
    const iso = dayOffset(offset)
    const d = new Date(`${iso}T12:00:00`)
    const weekday = d.toLocaleDateString('de-DE', { weekday: 'short' }).replace('.', '')
    const dayNum = d.toLocaleDateString('de-DE', { day: '2-digit', month: '2-digit' })
    return {
      iso,
      offset,
      label: offset === 0 ? 'Heute' : offset === -1 ? 'Gestern' : offset === 1 ? 'Morgen' : `${weekday} ${dayNum}`,
      isFuture: offset > 0,
    }
  })

  const focusedCaseId = $derived(hoveredCaseId ?? selectedCaseId)
  const focusedCase = $derived(cases.find((c) => c.id === focusedCaseId) ?? null)

  const filteredCases = $derived(
    selectedDay
      ? cases.filter((c) => c.events.some((e) => e.day === selectedDay))
      : cases,
  )

  const needsYou = $derived(filteredCases.filter((c) => c.status === 'needs_you'))
  const waiting = $derived(filteredCases.filter((c) => c.status === 'waiting'))
  const doneCases = $derived(filteredCases.filter((c) => c.status === 'done'))

  function eventsForDay(dayIso: string) {
    const out: { caseItem: CaseItem; label: string; phase: string }[] = []
    for (const c of cases) {
      for (const ev of c.events) {
        if (ev.day === dayIso) {
          out.push({ caseItem: c, label: ev.label, phase: ev.phase })
        }
      }
    }
    return out
  }

  function logoFor(domain: string | null) {
    void logoVersion
    return domain ? getBrandLogo(`info@${domain}`) : null
  }
</script>

<section class="board" aria-label="Vorgänge">
  <header class="board-head">
    <div>
      <h1>Vorgänge</h1>
      <p class="subtitle">
        {needsYou.length
          ? `${needsYou.length} Vorgang braucht dich · ${waiting.length} läuft`
          : `${waiting.length} laufende Vorgänge`}
      </p>
    </div>
    {#if selectedDay}
      <button type="button" class="pill-btn" onclick={() => (selectedDay = null)}>
        Tag-Filter zurücksetzen
      </button>
    {/if}
  </header>

  <div class="timeline-wrap" aria-label="Chronologie-Kalender">
    <div class="timeline" bind:this={timelineEl}>
      {#each calendarDays as day (day.iso)}
        {@const dayEvents = eventsForDay(day.iso)}
        {@const focusedEvents = focusedCase
          ? dayEvents.filter((e) => e.caseItem.id === focusedCase.id)
          : []}
        {@const isHighlighted = focusedEvents.length > 0}
        <button
          type="button"
          class="day-cell"
          class:today={day.iso === today}
          class:future={day.isFuture}
          class:selected={selectedDay === day.iso}
          class:highlighted={isHighlighted}
          onclick={() => (selectedDay = selectedDay === day.iso ? null : day.iso)}
        >
          <span class="day-label">{day.label}</span>
          <span class="day-dots">
            {#each dayEvents.slice(0, 4) as ev}
              <span
                class="ev-dot"
                class:due={ev.phase === 'due'}
                class:done={ev.phase === 'done'}
                class:active-dot={focusedCase?.id === ev.caseItem.id}
                title={`${ev.caseItem.title}: ${ev.label}`}
              ></span>
            {/each}
          </span>
          {#if focusedEvents.length > 0}
            <span class="day-focus-label">{focusedEvents[0].label}</span>
          {/if}
        </button>
      {/each}
    </div>
  </div>

  <div class="board-scroll">
    {#if needsYou.length > 0}
      <section class="group">
        <h2 class="group-title urgent">Braucht dich</h2>
        <div class="cards">
          {#each needsYou as c (c.id)}
            {@const expanded = selectedCaseId === c.id}
            {@const logo = logoFor(c.domain)}
            <article
              class="card needs-you"
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
                  {#if c.dueLabel || c.amount}
                    <span class="due-badge urgent">{c.amount ?? c.dueLabel}</span>
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
                  <div class="detail-events">
                    {#each c.events as ev (ev.id)}
                      <div class="detail-ev">
                        <span class="ev-date">{ev.day.slice(5).split('-').reverse().join('.')}</span>
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
                    <button type="button" class="act-btn" onclick={() => onchangestatus(c.id, 'aside')}>
                      Zu Nebenbei
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

    {#if waiting.length > 0}
      <section class="group">
        <h2 class="group-title">Wartet</h2>
        <div class="cards">
          {#each waiting as c (c.id)}
            {@const expanded = selectedCaseId === c.id}
            {@const logo = logoFor(c.domain)}
            <article
              class="card"
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
                  {#if c.dueLabel}
                    <span class="due-badge">{c.dueLabel}</span>
                  {/if}
                </div>
                <p class="next-step">{c.nextStep}</p>
                <div class="mini-steps" aria-hidden="true">
                  {#each c.events as ev, i}
                    <span class="step-pill" class:due={ev.phase === 'due'}>{ev.label}</span>
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
                  <div class="detail-events">
                    {#each c.events as ev (ev.id)}
                      <div class="detail-ev">
                        <span class="ev-date">{ev.day.slice(5).split('-').reverse().join('.')}</span>
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
                    <button type="button" class="act-btn" onclick={() => onchangestatus(c.id, 'aside')}>
                      Zu Nebenbei
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

    {#if categories.length > 0}
      <section class="group aside-group">
        <h2 class="group-title muted">Nebenbei · Wiederkehrende Kategorien</h2>
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

    {#if doneCases.length > 0}
      <section class="group done-group">
        <button type="button" class="collapse-btn" onclick={() => (showDone = !showDone)}>
          <Icon name={showDone ? 'chevronDown' : 'chevronRight'} size={14} />
          <span>Erledigt ({doneCases.length})</span>
        </button>
        {#if showDone}
          <div class="cards muted-cards">
            {#each doneCases as c (c.id)}
              <article
                class="card done-card"
                onmouseenter={() => onhovercase(c.id)}
                onmouseleave={() => onhovercase(null)}
              >
                <button
                  type="button"
                  class="card-main"
                  onclick={() => onselectcase(selectedCaseId === c.id ? null : c.id)}
                >
                  <div class="card-top">
                    <div class="card-titles">
                      <span class="card-title">{c.title}</span>
                      <span class="card-counterparty">{c.counterparty}</span>
                    </div>
                    <span class="due-badge">Erledigt</span>
                  </div>
                  <p class="next-step">{c.nextStep}</p>
                </button>
              </article>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  </div>
</section>

<style>
  .board {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--bg);
    overflow: hidden;
  }
  .board-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 16px 20px 10px;
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
  .pill-btn {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: 99px;
    padding: 4px 10px;
    font-size: 12px;
    color: var(--accent-text);
  }
  .timeline-wrap {
    padding: 0 20px 10px;
    border-bottom: 1px solid var(--border);
  }
  .timeline {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 4px;
    scrollbar-width: thin;
  }
  .day-cell {
    flex: 0 0 auto;
    min-width: 72px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 11px;
    color: var(--text-muted);
    transition: border-color 0.12s ease, background 0.12s ease;
  }
  .day-cell.today {
    border-color: var(--highlight);
    color: var(--text);
    font-weight: 650;
  }
  .day-cell.future {
    border-style: dashed;
  }
  .day-cell.selected {
    background: var(--selection);
    border-color: var(--highlight);
    color: var(--text);
  }
  .day-cell.highlighted {
    background: var(--selection);
    border-color: var(--ochre);
    color: var(--text);
  }
  .day-dots {
    display: flex;
    gap: 4px;
    min-height: 7px;
    align-items: center;
  }
  .ev-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-fill);
    opacity: 0.55;
  }
  .ev-dot.due {
    background: var(--ochre);
  }
  .ev-dot.done {
    background: var(--text-muted);
  }
  .ev-dot.active-dot {
    opacity: 1;
    transform: scale(1.25);
  }
  .day-focus-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--accent-text);
    white-space: nowrap;
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
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 0;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--brand-wash) 86%, transparent) 0%,
      color-mix(in srgb, var(--brand-wash) 35%, transparent) 58px,
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
    padding: 0 14px 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 10px;
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
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .detail-ev {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .ev-date {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    min-width: 42px;
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
  .done-card {
    opacity: 0.72;
  }
</style>

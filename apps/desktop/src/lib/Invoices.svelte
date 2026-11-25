<script lang="ts">
  // "Rechnungen": all invoices in one place, with a manual paid checkmark and a
  // per-year private/business split for the tax return. Takes no props, loads its own data.
  import { onMount } from 'svelte'
  import { openLink } from './api'
  import Icon from './Icon.svelte'
  import {
    accountKinds,
    exportInvoices,
    formatMoney,
    formatTotals,
    dueText,
    initials,
    invoiceYears,
    invoices,
    issuedText,
    monthKey,
    monthName,
    onInvoicesChanged,
    openInvoiceFile,
    setAccountKind,
    setInvoiceKind,
    setInvoicePaid,
    type AccountKind,
    type AccountKindInfo,
    type InvoiceExportReport,
    type InvoiceRecord,
  } from './invoices'

  const currentYear = new Date().getFullYear()

  let years = $state<number[]>([currentYear])
  let year = $state(currentYear)
  let items = $state<InvoiceRecord[]>([])
  let loaded = $state(false)
  let kindFilter = $state<'all' | AccountKind>('all')
  let missingOnly = $state(false)
  let accounts = $state<AccountKindInfo[]>([])
  let accountsOpen = $state(false)
  let error = $state<string | null>(null)
  let report = $state<InvoiceExportReport | null>(null)
  let exporting = $state(false)

  const kindLabel = (k: AccountKind) => (k === 'private' ? 'Privat' : 'Geschäftlich')
  const needsProof = (i: InvoiceRecord) => i.evidence === 'portal' || i.evidence === 'none'

  async function load() {
    try {
      items = await invoices(year)
    } catch (e) {
      error = `Rechnungen konnten nicht geladen werden: ${e}`
    }
    loaded = true
  }

  async function loadYears() {
    try {
      const ys = await invoiceYears()
      years = ys.includes(year) ? ys : [year, ...ys].sort((a, b) => b - a)
    } catch {
      /* the picker keeps the current year */
    }
  }

  async function loadAccounts() {
    try {
      accounts = await accountKinds()
    } catch {
      accounts = []
    }
  }

  onMount(() => {
    void loadYears()
    void load()
    void loadAccounts()
    let off: (() => void) | undefined
    let gone = false
    void onInvoicesChanged(() => {
      void load()
      void loadYears()
    }).then((f) => (gone ? f() : (off = f)))
    return () => {
      gone = true
      off?.()
    }
  })

  function pickYear(y: number) {
    year = y
    report = null
    missingOnly = false
    void load()
  }

  const visible = $derived(
    items.filter((i) => (kindFilter === 'all' || i.kind === kindFilter) && (!missingOnly || needsProof(i))),
  )
  const open = $derived(
    visible
      .filter((i) => !i.paid)
      .sort((a, b) => (a.due_at ?? Infinity) - (b.due_at ?? Infinity) || a.issued_at - b.issued_at),
  )
  const doneGroups = $derived.by(() => {
    const paid = visible.filter((i) => i.paid).sort((a, b) => b.issued_at - a.issued_at)
    const groups: { key: string; label: string; rows: InvoiceRecord[] }[] = []
    for (const i of paid) {
      const key = monthKey(i.issued_at)
      let g = groups[groups.length - 1]
      if (!g || g.key !== key) groups.push((g = { key, label: monthName(i.issued_at), rows: [] }))
      g.rows.push(i)
    }
    return groups
  })
  const missingCount = $derived(items.filter(needsProof).length)
  const privateList = $derived(items.filter((i) => i.kind === 'private'))
  const businessList = $derived(items.filter((i) => i.kind === 'business'))

  async function guarded(action: () => Promise<unknown>, revert?: () => void) {
    error = null
    try {
      await action()
    } catch (e) {
      revert?.()
      error = `Das hat nicht geklappt: ${e}`
    }
  }

  function togglePaid(i: InvoiceRecord) {
    const before = { paid: i.paid, source: i.paid_source, note: i.paid_note }
    const paid = !i.paid
    i.paid = paid
    i.paid_source = paid ? 'user' : null
    i.paid_note = null
    void guarded(
      () => setInvoicePaid(i.id, paid),
      () => {
        i.paid = before.paid
        i.paid_source = before.source
        i.paid_note = before.note
      },
    )
  }

  function setKind(i: InvoiceRecord, kind: AccountKind | null) {
    void guarded(async () => {
      await setInvoiceKind(i.id, kind)
      await load()
    })
  }

  function toggleKind(i: InvoiceRecord) {
    setKind(i, i.kind === 'private' ? 'business' : 'private')
  }

  function openProof(i: InvoiceRecord) {
    void guarded(() => openInvoiceFile(i.id))
  }

  function openPortal(url: string) {
    void guarded(() => openLink(url))
  }

  function changeAccount(a: AccountKindInfo, kind: AccountKind) {
    if (a.kind === kind) return
    const before = a.kind
    a.kind = kind
    void guarded(
      async () => {
        await setAccountKind(a.account_id, kind)
        await load()
      },
      () => (a.kind = before),
    )
  }

  async function doExport() {
    exporting = true
    report = null
    await guarded(async () => {
      report = await exportInvoices(year)
    })
    exporting = false
  }

  const totalsLine = $derived(
    `${year === currentYear ? `${year} bisher` : year}: ${items.length} ${items.length === 1 ? 'Rechnung' : 'Rechnungen'} · privat ${formatTotals(privateList)} · geschäftlich ${formatTotals(businessList)}`,
  )
</script>

{#snippet row(i: InvoiceRecord, done: boolean)}
  {@const due = done ? null : dueText(i.due_at)}
  <li class="row" class:done>
    <input
      type="checkbox"
      class="check"
      checked={i.paid}
      onchange={() => togglePaid(i)}
      aria-label={i.paid ? `${i.issuer}: wieder als offen markieren` : `${i.issuer}: als bezahlt markieren`}
    />
    <span class="tile" aria-hidden="true">{initials(i.issuer)}</span>
    <div class="main">
      <div class="line1">
        <span class="issuer">{i.issuer}</span>
        <span class="title">{i.title}</span>
      </div>
      <div class="line2">
        {#if done}
          <span>{i.paid_note ?? (i.paid_source === 'user' ? 'Von dir abgehakt' : 'Automatisch als bezahlt erkannt')}</span>
        {:else if due}
          <span class:overdue={due.overdue}>{#if due.overdue}<Icon name="warning" size={13} /> {/if}{due.text}</span>
        {/if}
        <span class="sep" aria-hidden="true">·</span>
        <span>{issuedText(i.issued_at)}</span>
        <button
          type="button"
          class="chip kind"
          class:business={i.kind === 'business'}
          onclick={() => toggleKind(i)}
          title="Klicken, um diese Rechnung {i.kind === 'private' ? 'als geschäftlich' : 'als privat'} zu führen"
        >
          {kindLabel(i.kind)}{#if i.kind_overridden}<span class="marker" title="Abweichend vom Konto"> ●</span>{/if}
        </button>
        {#if i.kind_overridden}
          <button type="button" class="link small" onclick={() => setKind(i, null)}>Wie Konto</button>
        {/if}
      </div>
    </div>
    <div class="side">
      <span class="amount">{i.amount_cents === null ? '–' : formatMoney(i.amount_cents, i.currency)}</span>
      <div class="proof">
        {#if i.evidence === 'attachment'}
          <button type="button" class="btn" onclick={() => openProof(i)}><Icon name="paperclip" size={14} /> PDF öffnen</button>
        {:else if i.evidence === 'mail'}
          <button type="button" class="btn" onclick={() => openProof(i)}><Icon name="mail" size={14} /> Mail als Beleg</button>
          {#if i.portal_url}
            <button type="button" class="link small" onclick={() => openPortal(i.portal_url!)}>{i.portal_label ?? 'Im Kundenportal ansehen'}</button>
          {/if}
        {:else if i.evidence === 'portal'}
          {#if i.portal_url}
            <button type="button" class="btn primary" onclick={() => openPortal(i.portal_url!)}>
              <Icon name="chevronRight" size={14} /> {i.portal_label ?? 'Im Kundenportal abrufen'}
            </button>
          {/if}
          <span class="hint">Kein PDF in der Mail</span>
        {:else}
          <span class="hint">Nachweis fehlt</span>
        {/if}
      </div>
    </div>
  </li>
{/snippet}

<section class="invoices">
  <div class="page">
    <header>
      <div class="head-row">
        <h1>Rechnungen</h1>
        <div class="head-actions">
          <label class="year">
            <span class="sr">Jahr</span>
            <select value={year} onchange={(e) => pickYear(Number(e.currentTarget.value))}>
              {#each years as y (y)}<option value={y}>{y}</option>{/each}
            </select>
          </label>
          <button type="button" class="btn" onclick={doExport} disabled={exporting || items.length === 0}>
            <Icon name="download" size={14} /> Jahr exportieren
          </button>
        </div>
      </div>

      {#if report}
        <p class="note" role="status">
          {report.exported} {report.exported === 1 ? 'Rechnung' : 'Rechnungen'} exportiert nach <code>{report.path}</code>.
          {#if report.without_proof > 0}
            Bei {report.without_proof} {report.without_proof === 1 ? 'Rechnung fehlt' : 'Rechnungen fehlt'} noch der Nachweis.
          {/if}
        </p>
      {/if}
      {#if error}<p class="note err" role="alert">{error}</p>{/if}

      <div class="controls">
        <div class="chips" role="group" aria-label="Filter">
          {#each [['all', 'Alle'], ['private', 'Privat'], ['business', 'Geschäftlich']] as [k, label] (k)}
            <button type="button" class="chip" class:active={kindFilter === k} aria-pressed={kindFilter === k} onclick={() => (kindFilter = k as typeof kindFilter)}>{label}</button>
          {/each}
        </div>
        <button type="button" class="link small" aria-expanded={accountsOpen} onclick={() => (accountsOpen = !accountsOpen)}>
          Konten zuordnen
        </button>
      </div>

      {#if accountsOpen}
        <div class="accounts">
          <p class="muted">Neue Rechnungen übernehmen die Zuordnung des Kontos.</p>
          {#each accounts as a (a.account_id)}
            <div class="acc">
              <span class="acc-mail">{a.email}</span>
              <div class="seg" role="group" aria-label="Zuordnung für {a.email}">
                <button type="button" class:on={a.kind === 'private'} aria-pressed={a.kind === 'private'} onclick={() => changeAccount(a, 'private')}>Privat</button>
                <button type="button" class:on={a.kind === 'business'} aria-pressed={a.kind === 'business'} onclick={() => changeAccount(a, 'business')}>Geschäftlich</button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </header>

    {#if loaded && items.length === 0}
      <div class="empty">
        <Icon name="archive" size={28} />
        <p>Hier sammelt Halcyon deine Rechnungen. Sobald eine eintrifft, erscheint sie hier.</p>
      </div>
    {:else}
      {#if missingCount > 0}
        <p class="missing">
          <button type="button" class="link" aria-pressed={missingOnly} onclick={() => (missingOnly = !missingOnly)}>
            {missingOnly
              ? 'Alle Rechnungen zeigen'
              : `Bei ${missingCount} ${missingCount === 1 ? 'Rechnung fehlt' : 'Rechnungen fehlt'} noch das PDF`}
          </button>
        </p>
      {/if}

      <h2>Noch offen</h2>
      {#if open.length}
        <ul class="list">{#each open as i (i.id)}{@render row(i, false)}{/each}</ul>
      {:else}
        <p class="muted">Nichts offen. Alles bezahlt.</p>
      {/if}

      {#if doneGroups.length}
        <h2 class="quiet">Erledigt</h2>
        {#each doneGroups as g (g.key)}
          <h3>{g.label}</h3>
          <ul class="list">{#each g.rows as i (i.id)}{@render row(i, true)}{/each}</ul>
        {/each}
      {/if}

      <footer>{totalsLine}</footer>
    {/if}
  </div>
</section>

<style>
  .invoices {
    height: 100%;
    overflow-y: auto;
    background: var(--bg);
    color: var(--text);
    container-type: inline-size;
  }
  .page {
    max-width: 960px;
    margin: 0 auto;
    padding: 32px 28px 64px;
  }
  .head-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  h1 {
    font-size: 32px;
    font-weight: 650;
    letter-spacing: -0.015em;
    margin: 0;
  }
  h2 {
    font-size: 20px;
    font-weight: 650;
    margin: 36px 0 12px;
  }
  h2.quiet,
  h3 {
    color: var(--text-muted);
  }
  h3 {
    font-size: 14px;
    font-weight: 500;
    margin: 16px 0 4px;
  }
  .head-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  select,
  .btn {
    font: inherit;
    font-size: 14px;
    color: var(--text);
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    border: 0;
    border-radius: 99px;
    padding: 8px 16px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.16s ease, transform 0.16s ease;
  }
  .btn:active:not(:disabled) {
    transform: scale(0.98);
  }
  /* Drawn by Halcyon instead of the toolkit, so it matches the pill buttons
     in both themes. */
  select {
    -webkit-appearance: none;
    appearance: none;
    padding-right: 34px;
  }
  select option {
    color: var(--text);
    background: var(--surface-raised);
  }
  .year {
    position: relative;
    display: inline-flex;
  }
  .year::after {
    content: '';
    position: absolute;
    right: 15px;
    top: 50%;
    width: 7px;
    height: 7px;
    margin-top: -5px;
    border-right: 2px solid currentColor;
    border-bottom: 2px solid currentColor;
    transform: rotate(45deg);
    color: var(--text-muted, var(--text));
    pointer-events: none;
  }
  @media (prefers-reduced-motion: reduce) {
    select,
    .btn {
      transition: none;
    }
    .btn:active:not(:disabled) {
      transform: none;
    }
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .btn:hover:not(:disabled),
  select:hover {
    background: color-mix(in srgb, var(--text) calc(var(--pill-mix) + 6%), transparent);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .btn.primary {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-fill);
    filter: brightness(1.12);
  }
  button:focus-visible,
  select:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--highlight);
    outline-offset: 2px;
  }
  .note {
    margin: 12px 0 0;
    color: var(--text-muted);
  }
  .note code {
    font-size: 13px;
    color: var(--text);
  }
  .note.err {
    color: var(--danger);
  }
  .controls {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 20px;
  }
  .chips {
    display: flex;
    gap: 6px;
  }
  .chip {
    font: inherit;
    font-size: 13px;
    color: var(--text-muted);
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 12px;
    cursor: pointer;
  }
  .chip:hover {
    background: var(--hover);
  }
  .chip.active {
    background: var(--selection);
    color: var(--text);
    border-color: var(--highlight);
  }
  .link {
    font: inherit;
    color: var(--accent-text);
    background: none;
    border: none;
    padding: 2px 0;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link.small {
    font-size: 13px;
  }
  .muted {
    color: var(--text-muted);
    margin: 0 0 8px;
  }
  .accounts {
    margin-top: 12px;
    padding: 12px 18px;
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
    border-radius: var(--card-radius-sm);
  }
  .acc {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding: 6px 0;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .seg button {
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-raised);
    border: none;
    padding: 4px 12px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .missing {
    margin: 24px 0 0;
    font-size: 14px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 6px 12px;
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight);
    border-radius: var(--card-radius-sm);
  }
  .row {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    gap: 12px;
    align-items: start;
    padding: 14px 4px;
    border-bottom: 1px solid color-mix(in srgb, var(--text) 8%, transparent);
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row.done {
    opacity: 0.78;
  }
  .check {
    width: 18px;
    height: 18px;
    margin: 4px 0 0;
    accent-color: var(--accent-fill);
    cursor: pointer;
  }
  .tile {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: var(--surface);
    color: var(--accent-text);
    font-size: 12px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .main {
    min-width: 0;
  }
  .line1 {
    display: flex;
    flex-wrap: wrap;
    column-gap: 8px;
  }
  .issuer {
    font-weight: 600;
  }
  .title {
    color: var(--text-muted);
  }
  .line2 {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    margin-top: 2px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .overdue {
    color: var(--danger);
    font-weight: 600;
  }
  .overdue :global(svg) {
    vertical-align: -2px;
  }
  .chip.kind {
    font-size: 12px;
    padding: 0 8px;
  }
  .chip.kind.business {
    border-style: dashed;
  }
  .marker {
    color: var(--highlight);
    font-size: 9px;
  }
  .side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
    text-align: right;
  }
  .amount {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }
  .proof {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
  }
  .hint {
    font-size: 13px;
    color: var(--text-muted);
  }
  .empty {
    margin-top: 64px;
    text-align: center;
    color: var(--text-muted);
  }
  .empty p {
    max-width: 380px;
    margin: 12px auto 0;
    font-size: 15px;
  }
  footer {
    margin-top: 32px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }
  @media (max-width: 760px) {
    .page {
      padding: 20px 16px 48px;
    }
    .row {
      grid-template-columns: auto 1fr auto;
    }
    .tile {
      display: none;
    }
  }
</style>

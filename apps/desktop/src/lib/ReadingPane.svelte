<script lang="ts">
  import { untrack } from 'svelte'
  import * as api from './api'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import Icon from './Icon.svelte'
  import { initials, longDate } from './format'
  import { isAppDark } from './theme.svelte'
  import { checkLink, holdsBack, ISSUE_TEXT, isStrong, type LinkIssue } from './phishing'
  import { isFlagged, isSeen, type MaskedPreview, type MessageSummary, type MessageView } from './types'

  let {
    message,
    ontoggleread,
    ontoggleflag,
    ondelete,
    onreply,
    hasJunk = false,
    onjunk,
    onacknowledge,
    embedded = false,
  }: {
    message: MessageSummary | null
    ontoggleread: (m: MessageSummary) => void
    ontoggleflag: (m: MessageSummary) => void
    ondelete: (m: MessageSummary) => void
    onreply: (m: MessageSummary, kind: 'reply' | 'reply_all' | 'forward') => void
    /** The account has a spam folder: the phishing banner offers to move the mail there. */
    hasJunk?: boolean
    onjunk?: (m: MessageSummary) => void
    /** "Ich kenne den Absender" (or taking it back); a local note only. */
    onacknowledge?: (m: MessageSummary, on: boolean) => void
    /** Inside another card (Heute): no pane chrome, compact toolbar, scrolls within its own max height. */
    embedded?: boolean
  } = $props()

  let view = $state<MessageView | null>(null)
  let loading = $state(false)
  let error = $state('')
  let allowRemote = $state(false)
  let notice = $state('')
  /** A link waiting for the user's decision: the real host, never just the text. */
  let linkWarning = $state<{
    href: string
    host: string
    claimed: string | null
    issues: LinkIssue[]
    heldMail: boolean
  } | null>(null)
  let frame: HTMLIFrameElement | undefined = $state()
  let otpCopied = $state(false)
  let maskedPreview = $state<MaskedPreview | null>(null)
  let maskLoading = $state(false)
  let imagePreview = $state<{ part: number; name: string; url: string } | null>(null)
  let imageLoading = $state<number | null>(null)

  const IMAGE_MIMES = new Set([
    'image/jpeg',
    'image/jpg',
    'image/png',
    'image/gif',
    'image/webp',
    'image/bmp',
    'image/avif',
  ])

  function isImageAttachment(a: { mime_type: string; name: string }): boolean {
    const mime = a.mime_type.trim().toLowerCase()
    if (IMAGE_MIMES.has(mime)) return true
    if (mime === 'application/octet-stream') {
      return /\.(jpe?g|png|gif|webp|bmp|avif)$/i.test(a.name.trim())
    }
    return false
  }

  const flagged = $derived(!!message && message.phishing.level !== 'none')
  const isNewsletter = $derived(!!message && message.header.list_unsubscribe.length > 0)
  const files = $derived(view?.attachments.filter((a) => !a.inline) ?? [])
  const canReplyAll = $derived(message ? hasOtherRecipients(message) : false)

  function extractEmail(raw: string): string {
    const match = raw.match(/<([^>]+)>/)
    return (match ? match[1] : raw).trim().toLowerCase()
  }

  function hasOtherRecipients(m: MessageSummary): boolean {
    const self = m.account_email.trim().toLowerCase()
    const from = (m.header.from_addr ?? '').trim().toLowerCase()
    const others = [...m.header.to, ...m.header.cc]
      .map(extractEmail)
      .filter((addr) => addr && addr !== self && addr !== from)
    return new Set(others).size > 0
  }

  const appDark = $derived(isAppDark())
  let logoVersion = $state(0)
  $effect(() => subscribeLogos(() => { logoVersion++ }))
  const senderLogo = $derived.by(() => {
    void logoVersion
    if (!message || message.folder_role === 'sent' || !message.header.dmarc_pass) return null
    return getBrandLogo(message.header.from_addr)
  })

  // Requests can overlap while flipping through messages; only the newest
  // result is shown.
  let seq = 0
  async function load(id: number, remote: boolean) {
    const mine = ++seq
    loading = true
    error = ''
    imagePreview = null
    try {
      const result = await api.messageContent(id, remote, appDark)
      if (mine === seq) view = result
    } catch (e) {
      if (mine === seq) {
        view = null
        error = String(e)
      }
    } finally {
      if (mine === seq) loading = false
    }
  }

  // A theme change re-renders the open message (dark design or light page).
  $effect(() => {
    void appDark
    untrack(() => {
      if (message && view) load(message.id, allowRemote)
    })
  })

  $effect(() => {
    const id = message?.id
    allowRemote = false
    notice = ''
    linkWarning = null
    view = null
    otpCopied = false
    maskedPreview = null
    if (id != null) load(id, false)
  })

  async function copyOtp(code: string) {
    await navigator.clipboard.writeText(code)
    otpCopied = true
    setTimeout(() => {
      otpCopied = false
    }, 2500)
  }

  async function toggleMaskPreview() {
    if (maskedPreview !== null) {
      maskedPreview = null
    } else if (message) {
      maskLoading = true
      try {
        maskedPreview = await api.maskMessagePreview(message.id)
      } catch (e) {
        notice = String(e)
      } finally {
        maskLoading = false
      }
    }
  }

  async function copyMasked() {
    if (!maskedPreview) return
    await navigator.clipboard.writeText(`${maskedPreview.masked_subject}\n\n${maskedPreview.masked_body}`)
    notice = 'Maskierter Text in die Zwischenablage kopiert.'
  }

  function loadImages() {
    if (!message) return
    allowRemote = true
    load(message.id, true)
  }

  async function alwaysLoadImages() {
    if (!message) return
    const addr = message.header.from_addr
    if (addr) {
      await api.trustRemoteSender(addr)
    }
    loadImages()
  }

  function openLink(url: string) {
    api.openLink(url).catch((e) => (notice = String(e)))
  }

  // The message document has no scripts of its own (sandbox without
  // allow-scripts, CSP script-src 'none'). Link clicks are handled here, so
  // links open in the system browser and deceptive ones get a warning.
  function onFrameLoad() {
    const doc = frame?.contentDocument
    if (!doc) return
    doc.addEventListener('click', (e) => {
      const a = (e.target as Element | null)?.closest?.('a')
      if (!a) return
      e.preventDefault()
      const href = a.getAttribute('href') ?? ''
      if (!href) return
      // Percent-encoded hosts are decoded before comparing with the text,
      // and a disguised target asks first even when the text shows no address.
      const check = checkLink(href, a.textContent ?? '')
      const heldMail = !!message && holdsBack(message)
      if (check && (check.claimed || check.issues.some(isStrong) || heldMail)) {
        linkWarning = { href, host: check.host, claimed: check.claimed, issues: check.issues, heldMail }
        return
      }
      openLink(href)
    })
  }

  async function save(part: number) {
    if (!message) return
    try {
      const path = await api.saveAttachment(message.id, part)
      notice = `Gespeichert: ${path}`
    } catch (e) {
      notice = String(e)
    }
  }

  async function openImagePreview(file: { part: number; name: string }) {
    if (!message || imageLoading !== null) return
    imageLoading = file.part
    try {
      const url = await api.previewImageAttachment(message.id, file.part)
      imagePreview = { part: file.part, name: file.name, url }
    } catch (e) {
      notice = String(e)
    } finally {
      imageLoading = null
    }
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  }
</script>

<article class="pane" class:embedded>
  {#if message}
    {@const h = message.header}
    <header class="head">
      <div class="actions" role="toolbar" aria-label="Aktionen">
        <button type="button" class="action" onclick={() => onreply(message, 'reply')} title="Antworten (R)">
          <Icon name="reply" size={16} />
          <span>Antworten</span>
        </button>
        {#if canReplyAll}
          <button
            type="button"
            class="action icon-only"
            onclick={() => onreply(message, 'reply_all')}
            title="Allen antworten (A)"
            aria-label="Allen antworten"
          >
            <Icon name="replyall" size={16} />
          </button>
        {/if}
        <button
          type="button"
          class="action icon-only"
          onclick={() => onreply(message, 'forward')}
          title="Weiterleiten (F)"
          aria-label="Weiterleiten"
        >
          <Icon name="forward" size={16} />
        </button>
        <span class="sep" aria-hidden="true"></span>
        <button
          type="button"
          class="action"
          onclick={() => ontoggleread(message)}
          title={isSeen(message) ? 'Als ungelesen markieren (U)' : 'Als gelesen markieren (U)'}
        >
          <Icon name={isSeen(message) ? 'mail' : 'mailopen'} size={16} />
          <span>{isSeen(message) ? 'Ungelesen' : 'Gelesen'}</span>
        </button>
        <button
          type="button"
          class="action icon-only"
          class:on={isFlagged(message)}
          onclick={() => ontoggleflag(message)}
          title="Markieren (S)"
          aria-label="Markieren"
          aria-pressed={isFlagged(message)}
        >
          <Icon name="star" size={16} filled={isFlagged(message)} />
        </button>
        <button
          type="button"
          class="action icon-only"
          onclick={() => ondelete(message)}
          title={message.folder_role === 'trash' ? 'Endgültig löschen (Entf)' : 'In den Papierkorb (Entf)'}
          aria-label={message.folder_role === 'trash' ? 'Endgültig löschen' : 'Löschen'}
        >
          <Icon name="trash" size={16} />
        </button>
        <span class="sep" aria-hidden="true"></span>
        <button
          type="button"
          class="action"
          class:on={maskedPreview !== null}
          onclick={toggleMaskPreview}
          disabled={maskLoading}
          title="Lokale PII-Maskierung und Vertraulichkeitsprüfung anzeigen"
        >
          <Icon name="shield" size={16} />
          <span>Maskiert</span>
        </button>
      </div>
      {#if flagged}
        {#if message.phishing.acknowledged}
          <div class="phish phish-note" role="note">
            <Icon name="warning" size={16} />
            <span>
              {message.phishing.level === 'phishing'
                ? 'Du kennst den Absender, die Mail bleibt aber zurückgehalten. Links fragen weiter nach.'
                : 'Du kennst den Absender. Links fragen weiter nach.'}
            </span>
            {#if onacknowledge}
              <button type="button" class="small-btn" onclick={() => onacknowledge(message, false)}>
                Hinweis wieder zeigen
              </button>
            {/if}
          </div>
        {:else}
          <div class="phish" class:strong={message.phishing.level === 'phishing'} role="alert">
            <div class="phish-head">
              <Icon name="warning" size={18} />
              <strong>
                {message.phishing.level === 'phishing'
                  ? 'Vorsicht: Das sieht nach Phishing aus'
                  : 'Diese Mail wirkt verdächtig'}
              </strong>
            </div>
            <ul class="phish-reasons">
              {#each message.phishing.reasons as r (r.code)}<li>{r.text}</li>{/each}
            </ul>
            <p class="phish-foot">Links in dieser Mail fragen vor dem Öffnen nach.</p>
            <div class="hint-actions">
              {#if hasJunk && onjunk && message.folder_role !== 'junk'}
                <button type="button" class="small-btn primary" onclick={() => onjunk(message)}>
                  In den Spam-Ordner
                </button>
              {/if}
              {#if onacknowledge}
                <button type="button" class="small-btn" onclick={() => onacknowledge(message, true)}>
                  Ich kenne den Absender
                </button>
              {/if}
            </div>
          </div>
        {/if}
      {/if}
      <h1>{h.subject ?? '(kein Betreff)'}</h1>

      {#if view?.otp_code}
        <div class="hint otp-banner">
          <Icon name="copy" size={16} />
          <span>Bestätigungscode erkannt: <strong class="otp-code">{view.otp_code}</strong></span>
          <div class="hint-actions">
            <button type="button" class="small-btn primary" onclick={() => copyOtp(view!.otp_code!)}>
              {otpCopied ? 'Kopiert ✓' : 'Code kopieren'}
            </button>
          </div>
        </div>
      {/if}
      {#if maskedPreview}
        <div class="mask-panel" role="region" aria-label="PII-maskierte Vorschau">
          <div class="mask-head">
            <span class="mask-badge" class:blocked={!maskedPreview.allowed}>
              <Icon name={maskedPreview.allowed ? 'shield' : 'warning'} size={14} />
              {maskedPreview.allowed
                ? `Für externe Analyse freigegeben (${maskedPreview.replacements} Platzhalter maskiert)`
                : `Vertraulich – lokal gesperrt: ${maskedPreview.confidential_reason ?? 'Schutzregel aktiv'}`}
            </span>
            <div class="hint-actions">
              <button type="button" class="small-btn" onclick={copyMasked}>
                <Icon name="copy" size={13} /> Kopieren
              </button>
              <button type="button" class="icon-btn" onclick={() => (maskedPreview = null)} aria-label="Maskierte Vorschau schließen">
                <Icon name="x" size={14} />
              </button>
            </div>
          </div>
          <div class="mask-subject"><strong>Betreff:</strong> {maskedPreview.masked_subject}</div>
          <pre class="mask-body">{maskedPreview.masked_body}</pre>
        </div>
      {/if}

      <div class="sender">
        {#if senderLogo}
          <div class="avatar brand" style:background={senderLogo.bg ?? undefined} aria-hidden="true">
            <img
              src={senderLogo.dataUrl}
              alt=""
              class:opaque={!senderLogo.padded}
              style:transform={senderLogo.scale !== 1 ? `scale(${senderLogo.scale})` : undefined}
            />
          </div>
        {:else}
          <div class="avatar" aria-hidden="true">{initials(h.from_name, h.from_addr)}</div>
        {/if}
        <div class="who">
          <div class="name">{h.from_name ?? h.from_addr}</div>
          {#if h.from_name}<div class="addr">{h.from_addr}</div>{/if}
        </div>
        <time>{longDate(h.date)}</time>
      </div>

      <dl class="details">
        {#if h.to.length}<dt>An</dt><dd>{h.to.join(', ')}</dd>{/if}
        {#if h.cc.length}<dt>Kopie</dt><dd>{h.cc.join(', ')}</dd>{/if}
        <dt>Konto</dt><dd>{message.account_email} · {message.folder_name}</dd>
      </dl>

      {#if isNewsletter}
        <div class="hint">
          <Icon name="unsubscribe" />
          <span>
            Newsletter{h.list_unsubscribe_post ? ' – Abmelden mit einem Klick möglich' : ''}.
            Das Abbestellen kommt in einem der nächsten Schritte.
          </span>
        </div>
      {/if}

      {#if view && view.rendered.remote_blocked > 0 && !allowRemote}
        <div class="hint images">
          <Icon name="image" />
          <span>
            {view.rendered.remote_blocked} externe
            {view.rendered.remote_blocked === 1 ? 'Inhalt' : 'Inhalte'} blockiert – sie würden dem
            Absender verraten, wann du die Mail liest.
          </span>
          <div class="hint-actions">
            <button type="button" class="small-btn" onclick={loadImages}>Bilder laden</button>
            {#if h.from_addr}
              <button type="button" class="small-btn" onclick={alwaysLoadImages}>
                Immer für {h.from_addr}
              </button>
            {/if}
          </div>
        </div>
      {/if}

      {#if files.length}
        <ul class="files" aria-label="Anhänge">
          {#each files as file (file.part)}
            {@const isImg = isImageAttachment(file)}
            <li>
              {#if isImg}
                <button
                  type="button"
                  class="file-preview-btn"
                  onclick={() => openImagePreview(file)}
                  disabled={imageLoading === file.part}
                  title="Bild direkt in Halcyon anzeigen"
                >
                  <Icon name="image" size={16} />
                  <span class="file-name">{file.name}</span>
                </button>
              {:else}
                <Icon name="paperclip" size={16} />
                <span class="file-name" title={file.name}>{file.name}</span>
              {/if}
              <span class="file-size">{formatSize(file.size)}</span>
              <button type="button" class="small-btn" onclick={() => save(file.part)}>
                <Icon name="download" size={14} />
                Speichern
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if notice}
        <div class="notice" role="status">
          <span>{notice}</span>
          <button
            type="button"
            class="icon-btn"
            onclick={() => (notice = '')}
            aria-label="Hinweis schließen"
          >
            <Icon name="x" size={14} />
          </button>
        </div>
      {/if}
    </header>

    <div class="body">
      {#if error}
        <div class="state err" role="alert">Inhalt konnte nicht geladen werden: {error}</div>
      {:else if !view}
        <div class="state">{loading ? 'Lädt …' : ''}</div>
      {:else}
        <!--
          sandbox without allow-scripts: nothing in the message can run.
          allow-same-origin only lets this component handle link clicks;
          it is safe because scripts stay disabled.
        -->
        <iframe
          bind:this={frame}
          title="Nachrichteninhalt"
          sandbox="allow-same-origin"
          referrerpolicy="no-referrer"
          srcdoc={view.rendered.document}
          class:dark={view.rendered.dark}
          onload={onFrameLoad}
        ></iframe>
      {/if}
    </div>

    {#if imagePreview}
      <div class="image-backdrop" role="presentation" onclick={() => (imagePreview = null)}>
        <div
          class="image-dialog"
          role="dialog"
          aria-modal="true"
          aria-label={imagePreview.name}
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => e.key === 'Escape' && (imagePreview = null)}
        >
          <div class="image-head">
            <span class="image-title" title={imagePreview.name}>{imagePreview.name}</span>
            <div class="image-actions">
              <button
                type="button"
                class="small-btn"
                onclick={() => imagePreview && save(imagePreview.part)}
              >
                <Icon name="download" size={14} />
                Speichern
              </button>
              <button
                type="button"
                class="icon-btn"
                onclick={() => (imagePreview = null)}
                aria-label="Bildvorschau schließen"
              >
                <Icon name="x" size={16} />
              </button>
            </div>
          </div>
          <div class="image-stage">
            <img src={imagePreview.url} alt={imagePreview.name} />
          </div>
        </div>
      </div>
    {/if}

    {#if linkWarning}
      {@const w = linkWarning}
      {@const disguised = w.issues.some(isStrong)}
      <div class="warn-backdrop" role="presentation" onclick={() => (linkWarning = null)}>
        <div
          class="warn-box"
          role="alertdialog"
          aria-labelledby="link-warn-title"
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => e.key === 'Escape' && (linkWarning = null)}
        >
          <h2 id="link-warn-title">
            <Icon name="warning" />
            {#if disguised}
              Der Link verbirgt sein Ziel
            {:else if w.claimed}
              Link führt woanders hin
            {:else}
              Link in einer verdächtigen Mail
            {/if}
          </h2>
          {#if w.host}
            <p>Der Link öffnet:</p>
            <p class="real-host">{w.host}</p>
          {/if}
          {#if w.claimed}
            <p>Angezeigt wird dagegen <strong>{w.claimed}</strong>.</p>
          {/if}
          {#if w.issues.length}
            <ul class="issues">
              {#each w.issues as issue (issue)}<li>{ISSUE_TEXT[issue]}</li>{/each}
            </ul>
          {/if}
          <p class="target">{w.href}</p>
          <p class="muted">
            {#if w.heldMail && !disguised && !w.claimed}
              Diese Mail wirkt verdächtig. Öffne den Link nur, wenn du dem Absender traust.
            {:else}
              So arbeiten Phishing-Mails. Öffne den Link nur, wenn du dem Absender vertraust.
            {/if}
          </p>
          <div class="warn-actions">
            <button type="button" class="small-btn primary" onclick={() => (linkWarning = null)}>
              Nicht öffnen
            </button>
            {#if !w.issues.includes('script')}
              <button
                type="button"
                class="small-btn"
                onclick={() => {
                  const t = linkWarning?.href
                  linkWarning = null
                  if (t) openLink(t)
                }}
              >
                Trotzdem öffnen
              </button>
            {/if}
          </div>
        </div>
      </div>
    {/if}
  {:else}
    <div class="nothing">
      <img src="/halcyon.svg" alt="" width="72" height="72" />
      <p>Wähle eine Nachricht aus.</p>
      <p class="keys"><kbd>↑</kbd> <kbd>↓</kbd> blättern · <kbd>/</kbd> suchen</p>
    </div>
  {/if}
</article>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface-raised);
    position: relative;
  }
  .sep {
    width: 1px;
    align-self: stretch;
    margin: 2px 4px;
    background: var(--border);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 14px;
  }
  .action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 30px;
    padding: 5px 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 13px;
    font-weight: 600;
  }
  .action.icon-only {
    padding: 5px 8px;
  }
  .action:hover {
    background: var(--hover);
  }
  .action :global(svg) {
    color: var(--accent-text);
  }
  .action.on :global(svg) {
    color: var(--ochre);
  }
  .head {
    padding: 16px 28px 12px;
    flex: none;
    overflow-y: auto;
    max-height: 55%;
  }
  h1 {
    margin: 0 0 16px;
    font-size: 21px;
    font-weight: 650;
    line-height: 1.3;
  }
  .sender {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .avatar {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-weight: 650;
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .avatar.brand {
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .avatar.brand img {
    width: 28px;
    height: 28px;
    object-fit: contain;
    display: block;
  }
  .avatar.brand img.opaque {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
  }
  .addr,
  time {
    color: var(--text-muted);
    font-size: 13px;
  }
  .details {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 14px;
    margin: 14px 0 10px;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .hint {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 10px;
    align-items: center;
    padding: 8px 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--ochre) 22%, transparent);
    border: 1px solid color-mix(in srgb, var(--ochre) 55%, transparent);
    font-size: 13px;
    margin-bottom: 8px;
  }
  .hint span {
    flex: 1 1 220px;
    min-width: 0;
  }
  .hint.images {
    background: color-mix(in srgb, var(--blue-green) 12%, transparent);
    border-color: color-mix(in srgb, var(--blue-green) 45%, transparent);
  }
  .hint-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-width: 100%;
  }
  .small-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    padding: 4px 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small-btn:hover {
    background: var(--hover);
  }
  .small-btn.primary {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
    border-color: transparent;
  }
  .files {
    list-style: none;
    margin: 4px 0 8px;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .files li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    font-size: 13px;
    max-width: 100%;
  }
  .files :global(svg) {
    color: var(--accent-text);
    flex: none;
  }
  .file-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .file-size {
    color: var(--text-muted);
    white-space: nowrap;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-muted);
    padding: 4px 0;
  }
  .notice span {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .icon-btn {
    border: 0;
    background: none;
    color: var(--text-muted);
    padding: 2px;
  }
  .body {
    flex: 1;
    min-height: 0;
    padding: 0 20px 20px;
    display: flex;
  }
  iframe {
    flex: 1;
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    /* Mails without their own dark design stay on a light page. */
    background: #ffffff;
    color-scheme: light;
  }
  iframe.dark {
    background: #1d1b1a;
    color-scheme: dark;
  }
  .state {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
  }
  .state.err {
    color: var(--danger);
    padding: 20px;
    text-align: center;
  }
  .warn-backdrop {
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--pitch-black) 40%, transparent);
    display: grid;
    place-items: center;
    padding: 20px;
  }
  .warn-box {
    max-width: 460px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 18px 20px;
    box-shadow: 0 12px 40px color-mix(in srgb, var(--pitch-black) 30%, transparent);
  }
  .warn-box h2 {
    margin: 0 0 10px;
    font-size: 16px;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .warn-box h2 :global(svg) {
    color: var(--ochre);
  }
  .warn-box p {
    margin: 6px 0;
    font-size: 13px;
  }
  .real-host {
    margin: 2px 0 6px;
    font-size: 17px !important;
    font-weight: 650;
    overflow-wrap: anywhere;
  }
  .issues {
    margin: 6px 0;
    padding-left: 18px;
    font-size: 13px;
  }
  .issues li {
    margin: 2px 0;
  }
  /* Warning banner of the phishing check: ochre like the other hints (an
     area with a second feature, here the heading and the edge), not a red wall. */
  .phish {
    margin-bottom: 14px;
    padding: 12px 14px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--ochre) 20%, transparent);
    border: 1px solid color-mix(in srgb, var(--ochre) 55%, transparent);
    border-left-width: 4px;
    font-size: 13px;
  }
  .phish.strong {
    background: color-mix(in srgb, var(--ochre) 28%, transparent);
    border-color: var(--ochre);
  }
  .phish-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 15px;
  }
  .phish-head :global(svg) {
    flex: none;
    color: var(--ochre);
  }
  .phish-reasons {
    margin: 8px 0 6px;
    padding-left: 22px;
  }
  .phish-reasons li {
    margin: 3px 0;
  }
  .phish-foot {
    margin: 0 0 10px;
    color: var(--text-muted);
  }
  .phish-note {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 10px;
    align-items: center;
    padding: 6px 12px;
  }
  .phish-note :global(svg) {
    flex: none;
    color: var(--ochre);
  }
  .phish-note span {
    flex: 1 1 220px;
    min-width: 0;
  }
  .target {
    font-family: ui-monospace, monospace;
    font-size: 12px !important;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--surface);
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--text-muted);
  }
  .warn-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 14px;
  }
  .nothing {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    color: var(--text-muted);
  }
  .nothing img {
    opacity: 0.9;
    margin-bottom: 8px;
  }
  .keys {
    font-size: 12px;
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 4px;
  }
  .otp-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  }
  .otp-code {
    font-family: ui-monospace, monospace;
    font-size: 14px;
    letter-spacing: 0.05em;
    padding: 2px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .mask-panel {
    margin-bottom: 12px;
    padding: 14px 16px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mask-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .mask-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--success, #2e7d32);
  }
  .mask-badge.blocked {
    color: var(--danger);
  }
  .mask-subject {
    font-size: 13px;
    color: var(--text);
  }
  .mask-body {
    margin: 0;
    font-family: ui-monospace, monospace;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--surface);
    padding: 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    max-height: 200px;
    overflow-y: auto;
  }

  /* Embedded in a card (Heute): the card provides background and padding. The
     whole pane scrolls inside a bounded height; the message frame gets a fixed
     height so its own scrolling stays usable. */
  .pane.embedded {
    background: transparent;
    max-height: 60vh;
    overflow-y: auto;
  }
  .embedded .head {
    padding: 0 0 8px;
    max-height: none;
    overflow: visible;
  }
  .embedded .actions {
    gap: 4px;
    margin-bottom: 10px;
  }
  .embedded .action {
    min-height: 28px;
    padding: 3px 8px;
  }
  .embedded .action span {
    /* compact toolbar: only "Antworten" keeps its label */
    display: none;
  }
  .embedded .action:first-child span {
    display: inline;
  }
  .embedded h1 {
    margin: 0 0 10px;
    font-size: 16px;
  }
  .embedded .details {
    margin: 10px 0 6px;
  }
  .embedded .body {
    flex: none;
    padding: 0;
  }
  .embedded iframe {
    flex: none;
    height: max(320px, 45vh);
  }
  .embedded .state {
    min-height: 120px;
  }
  .embedded .warn-backdrop,
  .embedded .image-backdrop {
    /* the pane scrolls, so a pane-relative backdrop could end up off screen */
    position: fixed;
    z-index: 30;
  }
  .file-preview-btn {
    flex: 1;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent-text);
    text-align: left;
    cursor: pointer;
    font-weight: 500;
  }
  .file-preview-btn:hover:not(:disabled) .file-name {
    text-decoration: underline;
  }
  .file-preview-btn:disabled {
    opacity: 0.6;
    cursor: progress;
  }
  .image-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    padding: 24px;
    background: color-mix(in srgb, var(--pitch-black) 68%, transparent);
  }
  .image-dialog {
    display: flex;
    flex-direction: column;
    max-width: min(92vw, 1100px);
    max-height: 90vh;
    min-width: 280px;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    box-shadow: 0 18px 50px color-mix(in srgb, var(--pitch-black) 45%, transparent);
    overflow: hidden;
  }
  .image-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .image-title {
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .image-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .image-stage {
    padding: 16px;
    overflow: auto;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--pitch-black) 12%, var(--bg));
  }
  .image-stage img {
    max-width: 100%;
    max-height: calc(90vh - 90px);
    object-fit: contain;
    border-radius: 6px;
  }
</style>

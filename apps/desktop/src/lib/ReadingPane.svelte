<script lang="ts">
  import { untrack } from 'svelte'
  import * as api from './api'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import Icon from './Icon.svelte'
  import { initials, longDate } from './format'
  import { isAppDark } from './theme.svelte'
  import { isFlagged, isSeen, type MessageSummary, type MessageView } from './types'

  let {
    message,
    ontoggleread,
    ontoggleflag,
    ondelete,
    onreply,
  }: {
    message: MessageSummary | null
    ontoggleread: (m: MessageSummary) => void
    ontoggleflag: (m: MessageSummary) => void
    ondelete: (m: MessageSummary) => void
    onreply: (m: MessageSummary, kind: 'reply' | 'reply_all' | 'forward') => void
  } = $props()

  let view = $state<MessageView | null>(null)
  let loading = $state(false)
  let error = $state('')
  let allowRemote = $state(false)
  let notice = $state('')
  let linkWarning = $state<{ shown: string; target: string } | null>(null)
  let frame: HTMLIFrameElement | undefined = $state()

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
    if (id != null) load(id, false)
  })

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

  function hostOf(url: string): string | null {
    try {
      return new URL(url).hostname.replace(/^www\./, '').toLowerCase()
    } catch {
      return null
    }
  }

  /** The domain a link text pretends to lead to, e.g. "paypal.com/login". */
  function shownHost(text: string): string | null {
    const t = text.trim()
    const m = t.match(/^(?:https?:\/\/)?((?:[a-z0-9-]+\.)+[a-z]{2,})(?:[/:?#]\S*)?$/i)
    return m ? m[1].replace(/^www\./, '').toLowerCase() : null
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
      const real = hostOf(href)
      const shown = shownHost(a.textContent ?? '')
      if (shown && real && shown !== real && !real.endsWith(`.${shown}`)) {
        linkWarning = { shown: a.textContent?.trim() ?? shown, target: href }
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

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  }
</script>

<article class="pane">
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
      </div>
      <h1>{h.subject ?? '(kein Betreff)'}</h1>

      <div class="sender">
        {#if senderLogo}
          <div class="avatar brand" aria-hidden="true">
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
            <li>
              <Icon name="paperclip" size={16} />
              <span class="file-name" title={file.name}>{file.name}</span>
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

    {#if linkWarning}
      <div class="warn-backdrop" role="presentation" onclick={() => (linkWarning = null)}>
        <div
          class="warn-box"
          role="alertdialog"
          aria-labelledby="link-warn-title"
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => e.key === 'Escape' && (linkWarning = null)}
        >
          <h2 id="link-warn-title"><Icon name="warning" /> Link führt woanders hin</h2>
          <p>Angezeigt wird <strong>{linkWarning.shown}</strong>, tatsächlich öffnet sich:</p>
          <p class="target">{linkWarning.target}</p>
          <p class="muted">
            So arbeiten Phishing-Mails. Öffne den Link nur, wenn du dem Absender vertraust.
          </p>
          <div class="warn-actions">
            <button type="button" class="small-btn primary" onclick={() => (linkWarning = null)}>
              Nicht öffnen
            </button>
            <button
              type="button"
              class="small-btn"
              onclick={() => {
                const t = linkWarning?.target
                linkWarning = null
                if (t) openLink(t)
              }}
            >
              Trotzdem öffnen
            </button>
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
</style>

<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import * as api from './lib/api'
  import AccountDialog from './lib/AccountDialog.svelte'
  import Composer from './lib/Composer.svelte'
  import ContextMenu, { type MenuItem } from './lib/ContextMenu.svelte'
  import type { SidebarTarget } from './lib/Sidebar.svelte'
  import Icon from './lib/Icon.svelte'
  import MessageList from './lib/MessageList.svelte'
  import ReadingPane from './lib/ReadingPane.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { accountLabel } from './lib/format'
  import {
    type AccountInfo,
    isFlagged,
    isSeen,
    type Draft,
    type LiveState,
    type OutboxEvent,
    type ReplyKind,
    type MessageSummary,
    type Overview,
    type SyncUpdate,
    type View,
  } from './lib/types'

  let overview: Overview | null = $state(null)
  let view: View = $state({ kind: 'unified', role: 'inbox' })
  let messages: MessageSummary[] = $state([])
  let loading = $state(true)
  let selectedId: number | null = $state(null)
  let query = $state('')
  let searchInput: HTMLInputElement | undefined = $state()
  let syncing = $state(false)
  let syncStatus = $state('')
  let errors: string[] = $state([])
  let accountDialogOpen = $state(false)
  let live: Record<string, LiveState> = $state({})
  // Accounts being synced right now: by the full sync (one at a time) or by
  // a background watcher.
  let fullSyncEmail: string | null = $state(null)
  let watcherActive: Record<string, boolean> = $state({})
  const busyEmails = $derived([
    ...(fullSyncEmail ? [fullSyncEmail] : []),
    ...Object.keys(watcherActive).filter((e) => watcherActive[e]),
  ])

  // The sidebar animation lingers briefly after an account goes idle. A sync
  // often ends and the next begins right away (watcher after full sync, or
  // folder by folder); without this the animation would restart mid-sweep.
  const LINGER_MS = 1200
  let activeEmails: string[] = $state([])
  const lingerTimers = new Map<string, ReturnType<typeof setTimeout>>()
  $effect(() => {
    const busy = busyEmails
    untrack(() => {
      for (const email of busy) {
        clearTimeout(lingerTimers.get(email))
        lingerTimers.delete(email)
        if (!activeEmails.includes(email)) activeEmails = [...activeEmails, email]
      }
      for (const email of activeEmails) {
        if (busy.includes(email) || lingerTimers.has(email)) continue
        lingerTimers.set(
          email,
          setTimeout(() => {
            lingerTimers.delete(email)
            activeEmails = activeEmails.filter((e) => e !== email)
          }, LINGER_MS),
        )
      }
    })
  })
  const liveStates = $derived(Object.values(live))
  const liveOffline = $derived(liveStates.filter((s) => s.state === 'offline'))

  const selected = $derived(messages.find((m) => m.id === selectedId) ?? null)
  const searching = $derived(query.trim().length > 0)
  const title = $derived.by(() => {
    if (searching) return `Suche: ${query.trim()}`
    const v = view
    if (v.kind === 'unified') return 'Alle Posteingänge'
    const acc = overview?.accounts.find((a) => a.id === v.accountId)
    const label = acc ? accountLabel(acc.email) : null
    if (v.kind === 'folder') {
      return label ? `${v.folderName} · ${label}` : v.folderName
    }
    return label ? `Posteingang · ${label}` : 'Posteingang'
  })

  // Requests can overlap while typing; only the newest result is shown.
  let requestSeq = 0
  async function load() {
    const seq = ++requestSeq
    loading = true
    try {
      const result = searching ? await api.search(query.trim()) : await api.messages(view)
      if (seq !== requestSeq) return
      messages = result
      if (!result.some((m) => m.id === selectedId)) selectedId = null
    } catch (e) {
      errors = [...errors, String(e)]
    } finally {
      if (seq === requestSeq) loading = false
    }
  }

  async function refreshOverview() {
    try {
      overview = await api.overview()
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined
  function onSearchInput() {
    clearTimeout(searchTimer)
    searchTimer = setTimeout(load, 150)
  }

  function selectView(v: View) {
    view = v
    query = ''
    selectedId = null
    load()
  }

  let statusTimer: ReturnType<typeof setTimeout> | undefined
  function flashStatus(text: string, after: string) {
    clearTimeout(statusTimer)
    syncStatus = text
    statusTimer = setTimeout(() => (syncStatus = after), 8000)
  }

  function handleSync(u: SyncUpdate) {
    switch (u.kind) {
      case 'started':
        clearTimeout(statusTimer)
        syncing = true
        syncStatus = 'Synchronisiere …'
        errors = []
        break
      case 'account':
        syncStatus = accountLabel(u.email)
        fullSyncEmail = u.email
        break
      case 'folder':
        syncStatus = `${accountLabel(u.email)} · ${u.path} ${u.done}/${u.total}`
        break
      case 'bodies':
        syncStatus = `${accountLabel(u.email)} · lade Inhalte ${u.done}/${u.total}`
        break
      case 'failed':
        errors = [...errors, `${u.email ? accountLabel(u.email) + ': ' : ''}${u.error}`]
        break
      case 'finished': {
        syncing = false
        fullSyncEmail = null
        const time = new Date().toLocaleTimeString('de-DE', { hour: '2-digit', minute: '2-digit' })
        // The result of this run is shown briefly; afterwards only when it
        // ran, so the toolbar never shows a stale count.
        flashStatus(
          u.new_messages === 0
            ? 'Abgerufen – alles aktuell'
            : `Abgerufen – ${u.new_messages} neue Nachricht${u.new_messages === 1 ? '' : 'en'}`,
          `Zuletzt abgerufen um ${time}`,
        )
        refreshOverview()
        load()
        break
      }
    }
  }

  // ----- actions on messages ---------------------------------------------

  /** Applies a flag change locally at once; the backend syncs it later. */
  async function setFlag(ids: number[], flag: 'seen' | 'flagged', on: boolean) {
    const name = flag === 'seen' ? '\\Seen' : '\\Flagged'
    for (const m of messages) {
      if (!ids.includes(m.id)) continue
      const flags = m.header.flags.filter((f) => f !== name)
      if (on) flags.push(name)
      m.header.flags = flags
    }
    try {
      await api.setFlag(ids, flag, on)
      if (flag === 'seen') refreshOverview()
    } catch (e) {
      errors = [...errors, String(e)]
      load()
    }
  }

  function toggleRead(m: MessageSummary) {
    setFlag([m.id], 'seen', !isSeen(m))
  }

  function toggleFlag(m: MessageSummary) {
    setFlag([m.id], 'flagged', !isFlagged(m))
  }

  let confirmDelete: MessageSummary | null = $state(null)

  async function deleteMessage(m: MessageSummary, confirmed = false) {
    // Messages in the trash are deleted for good: ask first.
    if (m.folder_role === 'trash' && !confirmed) {
      confirmDelete = m
      return
    }
    confirmDelete = null
    const index = messages.findIndex((x) => x.id === m.id)
    try {
      await api.deleteMessages([m.id])
      messages = messages.filter((x) => x.id !== m.id)
      if (selectedId === m.id) {
        selectedId = messages[Math.min(index, messages.length - 1)]?.id ?? null
      }
      refreshOverview()
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  // ----- sidebar context menu ------------------------------------------------

  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null)
  let confirmEmpty: { folderId: number; name: string; account: string; total: number } | null =
    $state(null)

  async function markRead(folderIds: number[]) {
    try {
      await api.markFoldersRead(folderIds)
      refreshOverview()
      load()
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  async function emptyNow(folderId: number) {
    confirmEmpty = null
    try {
      const n = await api.emptyFolder(folderId)
      showToast({ kind: 'done', text: `${n} Nachricht${n === 1 ? '' : 'en'} endgültig gelöscht.` }, 3000)
      refreshOverview()
      load()
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  function openContext(target: SidebarTarget, x: number, y: number) {
    const inboxes = (accs: AccountInfo[]) =>
      accs.flatMap((a) => a.folders.filter((f) => f.role === 'inbox' && f.unread > 0).map((f) => f.id))
    const sync: MenuItem = { label: 'Jetzt abrufen', icon: 'sync', action: () => api.startSync() }
    let items: MenuItem[]
    if (target.kind === 'unified') {
      const ids = inboxes(overview?.accounts ?? [])
      items = [
        { label: 'Alle als gelesen markieren', icon: 'mailopen', disabled: !ids.length, action: () => markRead(ids) },
        sync,
      ]
    } else if (target.kind === 'account') {
      const ids = inboxes([target.account])
      items = [
        { label: 'Posteingang als gelesen markieren', icon: 'mailopen', disabled: !ids.length, action: () => markRead(ids) },
        sync,
        { label: 'Konto bearbeiten …', icon: 'pencil', action: () => (accountDialogOpen = true) },
      ]
    } else {
      const f = target.folder
      items = [
        { label: 'Alle als gelesen markieren', icon: 'mailopen', disabled: f.unread === 0, action: () => markRead([f.id]) },
      ]
      if (f.role === 'junk' || f.role === 'trash') {
        items.push({
          label: f.role === 'junk' ? 'Spam leeren …' : 'Papierkorb leeren …',
          icon: 'trash',
          danger: true,
          disabled: f.total === 0,
          action: () =>
            (confirmEmpty = { folderId: f.id, name: f.name, account: accountLabel(target.account.email), total: f.total }),
        })
      }
    }
    menu = { x, y, items }
  }

  // ----- writing -----------------------------------------------------------

  let composing: Draft | null = $state(null)
  // Bumped for every new composer, so a replaced one starts fresh.
  let composerKey = $state(0)

  function openComposer(d: Draft) {
    composing = d
    composerKey++
  }

  function newMessage() {
    const accountId =
      view.kind === 'account' || view.kind === 'folder'
        ? view.accountId
        : (overview?.accounts[0]?.id ?? 0)
    openComposer({
      account_id: accountId,
      to: [],
      cc: [],
      bcc: [],
      subject: '',
      body: '',
      in_reply_to: null,
      references: [],
      reply_to_local_id: null,
    })
  }

  async function reply(m: MessageSummary, kind: ReplyKind) {
    try {
      const seed = await api.composeReply(m.id, kind)
      openComposer({
        account_id: seed.account_id,
        to: seed.draft.to,
        cc: seed.draft.cc,
        bcc: [],
        subject: seed.draft.subject,
        body: seed.draft.body,
        in_reply_to: seed.draft.in_reply_to,
        references: seed.draft.references,
        reply_to_local_id: kind === 'forward' ? null : m.id,
      })
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  // The toast at the bottom: undo window, sending, result.
  type Toast =
    | { kind: 'undo'; id: number; until: number; subject: string }
    | { kind: 'busy' | 'done' | 'info'; text: string }
    | { kind: 'failed'; id: number; text: string }
  let toast: Toast | null = $state(null)
  let now = $state(Date.now())
  let toastTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => {
    if (toast?.kind !== 'undo') return
    const tick = setInterval(() => (now = Date.now()), 250)
    return () => clearInterval(tick)
  })
  function showToast(t: Toast, hideAfterMs?: number) {
    clearTimeout(toastTimer)
    // The countdown reads `now`; start it from the current time.
    now = Date.now()
    toast = t
    if (hideAfterMs) toastTimer = setTimeout(() => (toast = null), hideAfterMs)
  }

  async function send(d: Draft) {
    const ticket = await api.sendMessage(d)
    composing = null
    showToast({ kind: 'undo', id: ticket.id, until: ticket.send_after * 1000, subject: d.subject })
  }

  async function undoSend(id: number) {
    const draft = await api.cancelSend(id)
    if (draft) {
      openComposer(draft)
      showToast({ kind: 'info', text: 'Senden abgebrochen.' }, 3000)
    } else {
      showToast({ kind: 'info', text: 'Zu spät – die Nachricht ist schon unterwegs.' }, 4000)
    }
  }

  async function editFailed(id: number) {
    const draft = await api.takeFailed(id)
    if (draft) openComposer(draft)
    toast = null
  }

  function handleOutbox(e: OutboxEvent) {
    switch (e.state) {
      case 'sending':
        showToast({ kind: 'busy', text: 'Wird gesendet …' })
        break
      case 'sent':
        showToast({ kind: 'done', text: 'Gesendet.' }, 3000)
        break
      case 'retrying':
        showToast(
          { kind: 'info', text: `Senden fehlgeschlagen (${e.error}). Neuer Versuch in ${e.retry_in} s.` },
          8000,
        )
        break
      case 'failed':
        showToast({ kind: 'failed', id: e.id, text: `„${e.subject}" wurde nicht gesendet: ${e.error}` })
        break
      case 'sent_copy_failed':
        errors = [...errors, `Gesendet, aber nicht unter „Gesendet" abgelegt: ${e.error}`]
        break
    }
  }

  // Opening a message marks it read – once per opening, so "mark unread"
  // on the open message sticks.
  let autoMarkedId: number | null = null
  $effect(() => {
    const id = selectedId
    untrack(() => {
      if (id === null || id === autoMarkedId) return
      autoMarkedId = id
      const m = messages.find((x) => x.id === id)
      if (m && !isSeen(m)) setFlag([m.id], 'seen', true)
    })
  })

  function moveSelection(delta: number) {
    if (messages.length === 0) return
    const index = messages.findIndex((m) => m.id === selectedId)
    const next = index < 0 ? 0 : Math.min(messages.length - 1, Math.max(0, index + delta))
    selectedId = messages[next].id
  }

  function onKeydown(e: KeyboardEvent) {
    if (accountDialogOpen || composing || menu || confirmEmpty) return
    const typing =
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      e.target instanceof HTMLSelectElement
    if (e.key === 'Escape' && typing) {
      query = ''
      searchInput?.blur()
      load()
      return
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) {
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault()
        searchInput?.focus()
      }
      return
    }
    if (e.key === '/') {
      e.preventDefault()
      searchInput?.focus()
    } else if (e.key === 'ArrowDown' || e.key === 'j') {
      e.preventDefault()
      moveSelection(1)
    } else if (e.key === 'ArrowUp' || e.key === 'k') {
      e.preventDefault()
      moveSelection(-1)
    } else if (e.key === 'c') {
      e.preventDefault()
      newMessage()
    } else if (selected && e.key === 'r') {
      e.preventDefault()
      reply(selected, 'reply')
    } else if (selected && e.key === 'a') {
      e.preventDefault()
      reply(selected, 'reply_all')
    } else if (selected && e.key === 'f') {
      e.preventDefault()
      reply(selected, 'forward')
    } else if (selected && e.key === 'u') {
      toggleRead(selected)
    } else if (selected && e.key === 's') {
      toggleFlag(selected)
    } else if (selected && (e.key === 'Delete' || e.key === '#')) {
      e.preventDefault()
      deleteMessage(selected)
    }
  }

  onMount(() => {
    refreshOverview()
    load()
    const unlisteners = [
      api.onSync(handleSync),
      api.onLive((s) => (live = { ...live, [s.email]: s })),
      api.onOutbox(handleOutbox),
      api.onActivity((a) => (watcherActive = { ...watcherActive, [a.email]: a.active })),
      // Background changes: refresh counts and the visible list, keeping the
      // selection (load() keeps it if the message still exists).
      api.onMailChanged(() => {
        refreshOverview()
        if (!searching) load()
      }),
    ]
    return () => {
      unlisteners.forEach((u) => u.then((f) => f()))
    }
  })
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app">
  <Sidebar
    {overview}
    {view}
    {activeEmails}
    onselect={selectView}
    onmanageaccounts={() => (accountDialogOpen = true)}
    oncontext={openContext}
  />

  <main>
    <div class="toolbar">
      <label class="search">
        <Icon name="search" size={16} />
        <input
          bind:this={searchInput}
          bind:value={query}
          oninput={onSearchInput}
          type="search"
          placeholder="Alle Konten durchsuchen …"
          aria-label="Suche"
          spellcheck="false"
        />
        {#if !searching}<kbd>/</kbd>{/if}
      </label>

      <div class="status" aria-live="polite">
        {#if api.isMock}<span class="mock">Testdaten</span>{/if}
        {#if liveStates.length}
          <span
            class="live"
            class:offline={liveOffline.length > 0}
            title={liveStates
              .map((s) =>
                s.state === 'push'
                  ? `${s.email}: live`
                  : s.state === 'polling'
                    ? `${s.email}: wird alle 5 Minuten geprüft`
                    : `${s.email}: offline – ${s.error}`,
              )
              .join('\n')}
          >
            <span class="dot"></span>
            {liveOffline.length ? `${liveOffline.length} offline` : 'Live'}
          </span>
        {/if}
        <span>{syncStatus}</span>
      </div>

      <button class="compose" onclick={newMessage} title="Neue Nachricht (C)">
        <Icon name="compose" />
        <span>Verfassen</span>
      </button>
      <button
        class="sync"
        class:spinning={syncing}
        onclick={() => api.startSync()}
        disabled={syncing}
        title="Alle Konten abrufen"
      >
        <Icon name="sync" />
        <span>Abrufen</span>
      </button>
    </div>

    {#if errors.length}
      <div class="errors" role="alert">
        <ul>
          {#each errors as e}<li>{e}</li>{/each}
        </ul>
        <button class="close" onclick={() => (errors = [])} aria-label="Fehler ausblenden">
          <Icon name="x" size={16} />
        </button>
      </div>
    {/if}

    <div class="content">
      <MessageList
        {title}
        {messages}
        {selectedId}
        {loading}
        showAccount={searching || view.kind === 'unified'}
        onselect={(id) => (selectedId = id)}
      />
      <div class="pane-slot">
        <ReadingPane
          message={selected}
          ontoggleread={toggleRead}
          ontoggleflag={toggleFlag}
          ondelete={(m) => deleteMessage(m)}
          onreply={reply}
        />
        {#if composing}
          {#key composerKey}
            <Composer
              initial={composing}
              accounts={overview?.accounts ?? []}
              onsend={send}
              onclose={() => (composing = null)}
            />
          {/key}
        {/if}
      </div>
    </div>
  </main>
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if confirmEmpty}
  {@const c = confirmEmpty}
  <div class="confirm-backdrop" role="presentation" onclick={() => (confirmEmpty = null)}>
    <div
      class="confirm-box"
      role="alertdialog"
      aria-labelledby="empty-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.key === 'Escape' && (confirmEmpty = null)}
    >
      <h2 id="empty-title">{c.name} leeren?</h2>
      <p>
        {c.total} Nachricht{c.total === 1 ? '' : 'en'} in „{c.name}" ({c.account}) werden endgültig gelöscht,
        auch auf dem Server. Das lässt sich nicht rückgängig machen.
      </p>
      <div class="confirm-actions">
        <button type="button" class="btn" onclick={() => (confirmEmpty = null)}>Abbrechen</button>
        <button type="button" class="btn danger" onclick={() => emptyNow(c.folderId)}>Endgültig löschen</button>
      </div>
    </div>
  </div>
{/if}

{#if toast}
  <div class="toast" class:failed={toast.kind === 'failed'} role="status" aria-live="polite">
    {#if toast.kind === 'undo'}
      {@const t = toast}
      {@const left = Math.max(0, Math.ceil((t.until - now) / 1000))}
      <span>„{t.subject || '(kein Betreff)'}" wird in {left} s gesendet.</span>
      <button type="button" class="toast-btn" onclick={() => undoSend(t.id)}>Rückgängig</button>
    {:else if toast.kind === 'failed'}
      {@const t = toast}
      <span>{t.text}</span>
      <button type="button" class="toast-btn" onclick={() => editFailed(t.id)}>Bearbeiten</button>
      <button type="button" class="toast-x" onclick={() => (toast = null)} aria-label="Schließen">
        <Icon name="x" size={14} />
      </button>
    {:else}
      <span>{toast.text}</span>
    {/if}
  </div>
{/if}

{#if confirmDelete}
  {@const m = confirmDelete}
  <div class="confirm-backdrop" role="presentation" onclick={() => (confirmDelete = null)}>
    <div
      class="confirm-box"
      role="alertdialog"
      aria-labelledby="confirm-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.key === 'Escape' && (confirmDelete = null)}
    >
      <h2 id="confirm-title">Endgültig löschen?</h2>
      <p>„{m.header.subject ?? '(kein Betreff)'}" liegt schon im Papierkorb und wird auch auf dem Server gelöscht. Das lässt sich nicht rückgängig machen.</p>
      <div class="confirm-actions">
        <button type="button" class="btn" onclick={() => (confirmDelete = null)}>Abbrechen</button>
        <button type="button" class="btn danger" onclick={() => deleteMessage(m, true)}>Endgültig löschen</button>
      </div>
    </div>
  </div>
{/if}

{#if accountDialogOpen}
  <AccountDialog
    onclose={() => (accountDialogOpen = false)}
    onchanged={async () => {
      await refreshOverview()
      await load()
    }}
  />
{/if}

<style>
  .pane-slot {
    position: relative;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .pane-slot > :global(.pane) {
    flex: 1;
  }
  .compose {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-weight: 600;
  }
  .compose:hover {
    background: var(--hover);
  }
  .compose :global(svg) {
    color: var(--accent-text);
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    z-index: 30;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(640px, calc(100vw - 32px));
    padding: 10px 14px;
    border-radius: 10px;
    background: var(--pitch-black);
    color: var(--snow);
    box-shadow: 0 8px 30px color-mix(in srgb, var(--pitch-black) 35%, transparent);
    font-size: 13px;
  }
  .toast.failed {
    background: color-mix(in srgb, var(--danger) 85%, var(--pitch-black));
  }
  .toast-btn {
    border: 0;
    background: none;
    color: var(--blue-green);
    font-weight: 700;
    padding: 2px 4px;
  }
  .toast.failed .toast-btn {
    color: #ffffff;
    text-decoration: underline;
  }
  .toast-x {
    border: 0;
    background: none;
    color: inherit;
    opacity: 0.8;
  }
  .confirm-backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--pitch-black) 40%, transparent);
    z-index: 20;
  }
  .confirm-box {
    max-width: 440px;
    padding: 18px 20px;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    box-shadow: 0 12px 40px color-mix(in srgb, var(--pitch-black) 30%, transparent);
  }
  .confirm-box h2 {
    margin: 0 0 8px;
    font-size: 16px;
  }
  .confirm-box p {
    margin: 0;
    font-size: 13px;
  }
  .confirm-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 16px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-weight: 600;
  }
  .btn.danger {
    background: var(--danger);
    border-color: transparent;
    color: #ffffff;
  }
  .app {
    display: grid;
    grid-template-columns: 240px 1fr;
    height: 100%;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    flex: 0 1 460px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-muted);
  }
  .search:focus-within {
    border-color: var(--highlight);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--blue-green) 25%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .status {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: flex-end;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
    color: var(--accent-text);
  }
  .live .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--blue-green);
  }
  .live.offline {
    color: var(--text-muted);
  }
  .live.offline .dot {
    background: var(--ochre);
  }
  .mock {
    padding: 1px 8px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--ochre) 30%, transparent);
    color: var(--text);
    font-weight: 600;
  }
  .sync {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent-fill);
    color: var(--on-accent-fill);
    font-weight: 600;
  }
  .sync:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .sync:disabled {
    cursor: progress;
    opacity: 0.85;
  }
  .spinning :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinning :global(svg) {
      animation: none;
    }
  }
  .errors {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: var(--radius);
    border: 1px solid color-mix(in srgb, var(--danger) 50%, transparent);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: 13px;
  }
  .errors ul {
    flex: 1;
    margin: 0;
    padding-left: 16px;
  }
  .close {
    border: 0;
    background: none;
    color: var(--text-muted);
    padding: 2px;
  }
  .content {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(320px, 42%) 1fr;
  }
  .content > :global(*) {
    min-height: 0;
  }
</style>

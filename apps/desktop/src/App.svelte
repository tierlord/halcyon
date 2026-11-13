<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import * as api from './lib/api'
  import AccountDialog from './lib/AccountDialog.svelte'
  import Icon from './lib/Icon.svelte'
  import MessageList from './lib/MessageList.svelte'
  import ReadingPane from './lib/ReadingPane.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { accountLabel } from './lib/format'
  import { isFlagged, isSeen, type MessageSummary, type Overview, type SyncUpdate, type View } from './lib/types'

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

  const selected = $derived(messages.find((m) => m.id === selectedId) ?? null)
  const searching = $derived(query.trim().length > 0)
  const title = $derived.by(() => {
    if (searching) return `Suche: ${query.trim()}`
    const v = view
    if (v.kind === 'unified') return 'Alle Posteingänge'
    const acc = overview?.accounts.find((a) => a.id === v.accountId)
    return acc ? `Posteingang · ${accountLabel(acc.email)}` : 'Posteingang'
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

  function handleSync(u: SyncUpdate) {
    switch (u.kind) {
      case 'started':
        syncing = true
        syncStatus = 'Synchronisiere …'
        errors = []
        break
      case 'account':
        syncStatus = accountLabel(u.email)
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
      case 'finished':
        syncing = false
        syncStatus =
          u.new_messages === 0
            ? 'Alles aktuell'
            : `${u.new_messages} neue Nachricht${u.new_messages === 1 ? '' : 'en'}`
        refreshOverview()
        load()
        break
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
    if (accountDialogOpen) return
    const typing = e.target instanceof HTMLInputElement
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
    const unlisten = api.onSync(handleSync)
    return () => {
      unlisten.then((f) => f())
    }
  })
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app">
  <Sidebar
    {overview}
    {view}
    onselect={selectView}
    onmanageaccounts={() => (accountDialogOpen = true)}
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
        <span>{syncStatus}</span>
      </div>

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
      <ReadingPane
        message={selected}
        ontoggleread={toggleRead}
        ontoggleflag={toggleFlag}
        ondelete={(m) => deleteMessage(m)}
      />
    </div>
  </main>
</div>

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

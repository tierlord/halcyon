<script lang="ts">
  import { onMount } from 'svelte'
  import { cubicOut } from 'svelte/easing'
  import { slide } from 'svelte/transition'
  import Icon from './Icon.svelte'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import { accountLabel, listDate } from './format'
  import { isFlagged, isSeen, type MessageSummary } from './types'

  let {
    title,
    messages,
    selectedId,
    loading,
    showAccount,
    highlightedIds = [],
    chatMode = false,
    sentReplies = new Map<string, MessageSummary[]>(),
    onselect,
    onclosecase,
  }: {
    title: string
    messages: MessageSummary[]
    selectedId: number | null
    loading: boolean
    showAccount: boolean
    highlightedIds?: number[]
    chatMode?: boolean
    sentReplies?: Map<string, MessageSummary[]>
    onselect: (id: number) => void
    onclosecase?: () => void
  } = $props()

  let listEl: HTMLUListElement | undefined = $state()
  let logoVersion = $state(0)
  let expandedThreadKeys = $state<Set<string>>(new Set())

  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const EXPAND_MS = reduceMotion ? 0 : 180

  onMount(() =>
    subscribeLogos(() => {
      logoVersion += 1
    }),
  )

  // Keep the selected row visible during keyboard navigation.
  $effect(() => {
    if (selectedId == null || !listEl) return
    listEl.querySelector(`[data-id="${selectedId}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  interface ThreadGroup {
    key: string
    items: MessageSummary[]
    latest: MessageSummary
    isThread: boolean
    hasUnread: boolean
    hasFlagged: boolean
    hasAttachments: boolean
    participants: string
  }

  function normMsgId(raw: string | null | undefined): string | null {
    if (!raw) return null
    const cleaned = raw.trim().replace(/^<|>$/g, '').toLowerCase()
    return cleaned || null
  }

  function cleanSubject(raw: string | null | undefined): string {
    if (!raw) return '(kein Betreff)'
    let s = raw.trim()
    while (/^(re|aw|wg|fw|fwd)\s*:\s*/i.test(s)) {
      s = s.replace(/^(re|aw|wg|fw|fwd)\s*:\s*/i, '').trim()
    }
    return s || '(kein Betreff)'
  }

  function senderLabel(m: MessageSummary): string {
    if (m.folder_role === 'sent') return 'Du'
    return m.header.from_name?.trim() || m.header.from_addr || 'Unbekannt'
  }

  const threadGroups = $derived.by((): ThreadGroup[] => {
    if (chatMode) return []
    const n = messages.length
    if (n === 0) return []

    // Union-Find to group messages connected by Message-ID / In-Reply-To / References.
    const parent = Array.from({ length: n }, (_, i) => i)
    const find = (x: number): number => {
      while (parent[x] !== x) {
        parent[x] = parent[parent[x]]
        x = parent[x]
      }
      return x
    }
    const union = (a: number, b: number) => {
      const ra = find(a)
      const rb = find(b)
      if (ra !== rb) parent[rb] = ra
    }

    const byRfcId = new Map<string, number>()
    for (let i = 0; i < n; i++) {
      const mid = normMsgId(messages[i].header.message_id)
      if (mid && !byRfcId.has(mid)) {
        byRfcId.set(mid, i)
      }
    }

    const byRefToken = new Map<string, number>()
    for (let i = 0; i < n; i++) {
      const h = messages[i].header
      const tokens = [h.in_reply_to, ...h.references]
        .map(normMsgId)
        .filter((t): t is string => t != null)
      for (const tok of tokens) {
        const direct = byRfcId.get(tok)
        if (direct != null) union(i, direct)
        const prev = byRefToken.get(tok)
        if (prev != null) {
          union(i, prev)
        } else {
          byRefToken.set(tok, i)
        }
      }
    }

    const buckets = new Map<number, MessageSummary[]>()
    for (let i = 0; i < n; i++) {
      const root = find(i)
      const list = buckets.get(root) ?? []
      list.push(messages[i])
      buckets.set(root, list)
    }

    const groups: ThreadGroup[] = []
    for (const baseItems of buckets.values()) {
      const seenIds = new Set(baseItems.map((m) => m.id))
      const combined = [...baseItems]

      // Merge any sent replies whose In-Reply-To points into this thread.
      for (const m of baseItems) {
        const rawMid = m.header.message_id
        if (!rawMid) continue
        const reps = sentReplies.get(rawMid) ?? []
        for (const rep of reps) {
          if (!seenIds.has(rep.id)) {
            seenIds.add(rep.id)
            combined.push(rep)
          }
        }
      }

      combined.sort((a, b) => (a.header.date ?? 0) - (b.header.date ?? 0))
      const latest = combined[combined.length - 1]
      const names: string[] = []
      for (const m of combined) {
        const label = senderLabel(m)
        if (!names.includes(label)) names.push(label)
      }

      groups.push({
        key: `t-${combined[0].id}`,
        items: combined,
        latest,
        isThread: combined.length >= 2,
        hasUnread: combined.some((m) => !isSeen(m)),
        hasFlagged: combined.some((m) => isFlagged(m)),
        hasAttachments: combined.some((m) => m.has_attachments),
        participants: names.join(', '),
      })
    }

    groups.sort((a, b) => (b.latest.header.date ?? 0) - (a.latest.header.date ?? 0))
    return groups
  })

  function toggleThread(key: string) {
    const next = new Set(expandedThreadKeys)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    expandedThreadKeys = next
  }
</script>

<section class="list" aria-label={title}>
  <header>
    <div class="head-left">
      {#if chatMode && onclosecase}
        <button type="button" class="back-btn" onclick={onclosecase} title="Zurück zur normalen Liste">
          ←
        </button>
      {/if}
      <h1>{title}</h1>
    </div>
    <span class="meta">{loading ? 'Lädt …' : `${messages.length} Nachrichten`}</span>
  </header>

  {#if !loading && messages.length === 0}
    <p class="empty">Hier ist nichts. Herrlich ruhig.</p>
  {/if}

  <ul bind:this={listEl} class:chat-list={chatMode} role="listbox" aria-label={title}>
    {#if chatMode}
      {#each messages as m (m.id)}
        {@const unread = !isSeen(m)}
        {@const isOutgoing = m.folder_role === 'sent'}
        {@const logo = (void logoVersion, !isOutgoing && m.header.dmarc_pass ? getBrandLogo(m.header.from_addr) : null)}
        <li
          role="option"
          aria-selected={m.id === selectedId}
          data-id={m.id}
          class:selected={m.id === selectedId}
          class:unread
          class:case-highlighted={highlightedIds.includes(m.id)}
          class:chat-in={!isOutgoing}
          class:chat-out={isOutgoing}
        >
          <button class:has-brand={!!logo} onclick={() => onselect(m.id)}>
            <span class="dot" aria-label={unread ? 'ungelesen' : undefined}></span>
            {#if logo}
              <span class="brand" aria-hidden="true">
                <img
                  src={logo.dataUrl}
                  alt=""
                  class:opaque={!logo.padded}
                  style:transform={logo.scale !== 1 ? `scale(${logo.scale})` : undefined}
                />
              </span>
            {/if}
            <span class="from">
              {#if isOutgoing}Du · Gesendet{:else}{m.header.from_name ?? m.header.from_addr ?? 'Unbekannt'}{/if}
            </span>
            <time>
              {#if isFlagged(m)}<span class="flag" title="Markiert"><Icon name="star" size={12} filled /></span>{/if}
              {listDate(m.header.date)}
            </time>
            <span class="subject">
              {#if m.has_attachments}<Icon name="paperclip" size={13} />{/if}
              {m.header.subject ?? '(kein Betreff)'}{#if m.preview}<span class="preview"> – {m.preview}</span>{/if}
            </span>
          </button>
        </li>
      {/each}
    {:else}
      {#each threadGroups as group (group.key)}
        {@const m = group.latest}
        {@const containsSelected = group.items.some((item) => item.id === selectedId)}
        {@const isExpanded = group.isThread && (expandedThreadKeys.has(group.key) || containsSelected)}
        {@const isHighlighted = group.items.some((item) => highlightedIds.includes(item.id))}
        {@const brandSource = group.items.find((item) => item.folder_role !== 'sent' && item.header.dmarc_pass) ?? m}
        {@const logo = (void logoVersion, brandSource.folder_role !== 'sent' && brandSource.header.dmarc_pass ? getBrandLogo(brandSource.header.from_addr) : null)}

        {#if !group.isThread}
          {@const unread = !isSeen(m)}
          <li
            role="option"
            aria-selected={m.id === selectedId}
            data-id={m.id}
            class:selected={m.id === selectedId}
            class:unread
            class:case-highlighted={isHighlighted}
          >
            <button class:has-brand={!!logo} onclick={() => onselect(m.id)}>
              <span class="dot" aria-label={unread ? 'ungelesen' : undefined}></span>
              {#if logo}
                <span class="brand" aria-hidden="true">
                  <img
                    src={logo.dataUrl}
                    alt=""
                    class:opaque={!logo.padded}
                    style:transform={logo.scale !== 1 ? `scale(${logo.scale})` : undefined}
                  />
                </span>
              {/if}
              <span class="from">{m.header.from_name ?? m.header.from_addr ?? 'Unbekannt'}</span>
              <time>
                {#if isFlagged(m)}<span class="flag" title="Markiert"><Icon name="star" size={12} filled /></span>{/if}
                {listDate(m.header.date)}
              </time>
              <span class="subject">
                {#if m.has_attachments}<Icon name="paperclip" size={13} />{/if}
                {m.header.subject ?? '(kein Betreff)'}{#if m.preview}<span class="preview"> – {m.preview}</span>{/if}
              </span>
              {#if showAccount}<span class="account">{accountLabel(m.account_email)}</span>{/if}
            </button>
          </li>
        {:else}
          <li
            class="thread-item"
            class:expanded={isExpanded}
            class:unread={group.hasUnread}
            class:case-highlighted={isHighlighted}
          >
            <button
              type="button"
              class="thread-toggle"
              class:has-brand={!!logo}
              onclick={() => toggleThread(group.key)}
              aria-expanded={isExpanded}
              title={isExpanded ? 'Unterhaltung zuklappen' : 'Unterhaltung aufklappen'}
            >
              <span class="dot" aria-label={group.hasUnread ? 'ungelesen' : undefined}></span>
              <span class="thread-chevron" class:rotated={isExpanded} aria-hidden="true">
                <Icon name="chevronRight" size={13} />
              </span>
              {#if logo}
                <span class="brand" aria-hidden="true">
                  <img
                    src={logo.dataUrl}
                    alt=""
                    class:opaque={!logo.padded}
                    style:transform={logo.scale !== 1 ? `scale(${logo.scale})` : undefined}
                  />
                </span>
              {/if}
              <span class="from">{group.participants}</span>
              <time>
                {#if group.hasFlagged}<span class="flag" title="Markiert"><Icon name="star" size={12} filled /></span>{/if}
                {listDate(m.header.date)}
              </time>
              <span class="subject">
                {#if group.hasAttachments}<Icon name="paperclip" size={13} />{/if}
                {cleanSubject(m.header.subject)}{#if !isExpanded && m.preview}<span class="preview"> – {m.preview}</span>{/if}
              </span>
              {#if showAccount && !isExpanded}<span class="account">{accountLabel(m.account_email)}</span>{/if}
            </button>

            {#if !isExpanded}
              <div class="thread-stack-lip" aria-hidden="true"></div>
            {:else}
              <div class="thread-messages" transition:slide={{ duration: EXPAND_MS, easing: cubicOut }}>
                {#each group.items as item (item.id)}
                  {@const itemUnread = !isSeen(item)}
                  {@const isOutgoing = item.folder_role === 'sent'}
                  <button
                    type="button"
                    data-id={item.id}
                    class="thread-msg"
                    class:in={!isOutgoing}
                    class:out={isOutgoing}
                    class:selected={item.id === selectedId}
                    class:unread={itemUnread}
                    onclick={() => onselect(item.id)}
                  >
                    <div class="thread-msg-top">
                      <span class="thread-msg-sender">
                        {#if itemUnread}<span class="inline-dot" aria-label="ungelesen"></span>{/if}
                        {isOutgoing ? 'Du · Gesendet' : (item.header.from_name ?? item.header.from_addr ?? 'Unbekannt')}
                      </span>
                      <time>
                        {#if item.has_attachments}<Icon name="paperclip" size={11} />{/if}
                        {listDate(item.header.date)}
                      </time>
                    </div>
                    <div class="thread-msg-preview">
                      {item.preview ?? item.header.subject ?? '(kein Text)'}
                    </div>
                  </button>
                {/each}
              </div>
            {/if}
          </li>
        {/if}
      {/each}
    {/if}
  </ul>
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-right: 1px solid var(--border);
    background: var(--bg);
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 16px 18px 10px;
  }
  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }
  .empty {
    padding: 40px 18px;
    text-align: center;
    color: var(--text-muted);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 8px 12px;
    overflow-y: auto;
    flex: 1;
  }
  li > button {
    position: relative;
    overflow: hidden;
    width: 100%;
    display: grid;
    grid-template-columns: 12px 1fr auto;
    grid-template-areas:
      'dot from date'
      'sub-left subject account';
    column-gap: 10px;
    row-gap: 2px;
    align-items: center;
    padding: 9px 10px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    text-align: left;
  }
  li > button.has-brand {
    grid-template-columns: 12px 32px 1fr auto;
    grid-template-areas:
      'dot brand from date'
      'sub-left brand subject account';
  }
  .brand {
    grid-area: brand;
    position: relative;
    z-index: 1;
    width: 32px;
    height: 32px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    flex-shrink: 0;
  }
  .brand img {
    width: 22px;
    height: 22px;
    object-fit: contain;
    display: block;
  }
  .brand img.opaque {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: calc(var(--radius) - 1px);
  }
  .dot,
  .from,
  time,
  .subject,
  .account {
    position: relative;
    z-index: 1;
  }
  li > button:hover {
    background: var(--hover);
  }
  li.selected > button {
    background: var(--selection);
    box-shadow: inset 3px 0 0 var(--highlight);
  }
  .dot {
    grid-area: dot;
    justify-self: center;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .unread .dot {
    background: var(--unread);
  }
  .from {
    grid-area: from;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .unread .from,
  .unread .subject {
    font-weight: 650;
  }
  time {
    grid-area: date;
    justify-self: end;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .flag {
    color: var(--ochre);
    margin-right: 2px;
    vertical-align: -1px;
  }
  .unread time {
    color: var(--accent-text);
    font-weight: 600;
  }
  .subject {
    grid-area: subject;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }
  .unread .subject {
    color: var(--text);
  }
  .subject :global(svg) {
    vertical-align: -2px;
    color: var(--text-muted);
  }
  .preview {
    font-weight: 400;
    color: var(--text-muted);
  }
  .account {
    grid-area: account;
    justify-self: end;
    font-size: 11px;
    padding: 0 6px;
    border-radius: 99px;
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .head-left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .back-btn {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius);
    padding: 2px 8px;
    font-size: 13px;
    color: var(--accent-text);
  }
  .back-btn:hover {
    background: var(--hover);
  }
  li.case-highlighted > button,
  li.thread-item.case-highlighted {
    background: color-mix(in srgb, var(--highlight) 12%, transparent);
    box-shadow: inset 3px 0 0 var(--highlight);
  }

  /* Thread item: unified header with animated chevron + sliding body */
  .thread-item {
    margin: 3px 0 6px;
    transition: margin 180ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .thread-item.expanded {
    margin: 6px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    overflow: hidden;
  }
  .thread-item:not(.expanded) > .thread-toggle {
    border: 1px solid var(--border);
    background: var(--surface-raised);
  }
  .thread-item.expanded > .thread-toggle {
    border-radius: 0;
    border-bottom: 1px solid var(--border);
  }
  .thread-chevron {
    grid-area: sub-left;
    position: relative;
    z-index: 1;
    justify-self: center;
    display: inline-flex;
    color: var(--text-muted);
    transition: transform 180ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .thread-chevron.rotated {
    transform: rotate(90deg);
  }
  .thread-stack-lip {
    height: 5px;
    margin: 2px 10px 0;
    border-left: 1px solid var(--border);
    border-right: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    border-bottom-left-radius: 6px;
    border-bottom-right-radius: 6px;
    background: var(--surface);
  }
  .thread-messages {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
  }
  .thread-msg {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    text-align: left;
  }
  .thread-msg.in {
    max-width: 92%;
    margin-right: auto;
    background: var(--surface-raised);
  }
  .thread-msg.out {
    max-width: 86%;
    margin-left: auto;
    background: var(--selection);
  }
  .thread-msg:hover {
    border-color: var(--highlight);
  }
  .thread-msg.selected {
    border-color: var(--highlight);
    box-shadow: inset 3px 0 0 var(--highlight);
  }
  .thread-msg-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 11px;
  }
  .thread-msg-sender {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .thread-msg.unread .thread-msg-sender {
    font-weight: 700;
  }
  .inline-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--unread);
    flex-shrink: 0;
  }
  .thread-msg-preview {
    font-size: 12px;
    color: var(--text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* Case chat mode */
  ul.chat-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 6px;
  }
  li.chat-in > button {
    max-width: 92%;
    margin-right: auto;
    background: var(--surface-raised);
    border: 1px solid var(--border);
  }
  li.chat-out > button {
    max-width: 86%;
    margin-left: auto;
    background: var(--selection);
    border: 1px solid var(--border);
  }
</style>

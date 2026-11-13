<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { formatRecipients, parseRecipients } from './recipients'
  import type { AccountInfo, Draft } from './types'

  let {
    initial,
    accounts,
    onsend,
    onclose,
  }: {
    initial: Draft
    accounts: AccountInfo[]
    /** Resolves when queued; rejects with a message to show. */
    onsend: (draft: Draft) => Promise<void>
    onclose: () => void
  } = $props()

  // The composer works on its own copy; `initial` only seeds it.
  // svelte-ignore state_referenced_locally
  let accountId = $state(initial.account_id || accounts[0]?.id || 0)
  // svelte-ignore state_referenced_locally
  let to = $state(formatRecipients(initial.to))
  // svelte-ignore state_referenced_locally
  let cc = $state(formatRecipients(initial.cc))
  // svelte-ignore state_referenced_locally
  let bcc = $state(formatRecipients(initial.bcc))
  // svelte-ignore state_referenced_locally
  let showCopies = $state(initial.cc.length > 0 || initial.bcc.length > 0)
  // svelte-ignore state_referenced_locally
  let subject = $state(initial.subject)
  // svelte-ignore state_referenced_locally
  let body = $state(initial.body)
  let error = $state('')
  let sending = $state(false)
  let confirmDiscard = $state(false)
  let toInput: HTMLInputElement | undefined = $state()
  let bodyInput: HTMLTextAreaElement | undefined = $state()

  // svelte-ignore state_referenced_locally
  const original = JSON.stringify([to, cc, bcc, subject, body])
  const dirty = $derived(JSON.stringify([to, cc, bcc, subject, body]) !== original)

  function draft(): Draft | string {
    const parsed = [to, cc, bcc].map(parseRecipients)
    const bad = parsed.flatMap((p) => p.invalid)
    if (bad.length) return `Ungültige Adresse: ${bad.join(', ')}`
    const [t, c, b] = parsed.map((p) => p.recipients)
    if (t.length + c.length + b.length === 0) return 'Bitte mindestens einen Empfänger angeben.'
    return { ...initial, account_id: accountId, to: t, cc: c, bcc: b, subject, body }
  }

  async function send() {
    if (sending) return
    const d = draft()
    if (typeof d === 'string') {
      error = d
      return
    }
    if (!d.subject.trim() && !confirm('Ohne Betreff senden?')) return
    sending = true
    error = ''
    try {
      await onsend(d)
    } catch (e) {
      error = String(e)
      sending = false
    }
  }

  function close() {
    if (dirty && !confirmDiscard) {
      confirmDiscard = true
      return
    }
    onclose()
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault()
      send()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      close()
    }
  }

  onMount(() => {
    // Replies start in the text (above the quote), new mails in "An".
    if (initial.to.length && bodyInput) {
      bodyInput.focus()
      bodyInput.setSelectionRange(0, 0)
      bodyInput.scrollTop = 0
    } else {
      toInput?.focus()
    }
  })
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="composer"
  role="dialog"
  tabindex="-1"
  aria-label={subject ? `Verfassen: ${subject}` : 'Neue Nachricht'}
  onkeydown={onKeydown}
>
  <header>
    <h2>{subject.trim() || 'Neue Nachricht'}</h2>
    <button type="button" class="icon-btn" onclick={close} aria-label="Schließen">
      <Icon name="x" size={18} />
    </button>
  </header>

  <div class="fields">
    {#if accounts.length > 1}
      <label class="row">
        <span>Von</span>
        <select bind:value={accountId}>
          {#each accounts as a (a.id)}
            <option value={a.id}>{a.display_name ? `${a.display_name} <${a.email}>` : a.email}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="row">
      <span>An</span>
      <input bind:this={toInput} bind:value={to} type="text" spellcheck="false" autocomplete="off" />
      {#if !showCopies}
        <button type="button" class="link-btn" onclick={() => (showCopies = true)}>Kopie</button>
      {/if}
    </label>
    {#if showCopies}
      <label class="row">
        <span>Kopie</span>
        <input bind:value={cc} type="text" spellcheck="false" autocomplete="off" />
      </label>
      <label class="row">
        <span title="Blindkopie: für die anderen Empfänger unsichtbar">Blind</span>
        <input bind:value={bcc} type="text" spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    <label class="row">
      <span>Betreff</span>
      <input bind:value={subject} type="text" />
    </label>
  </div>

  <textarea bind:this={bodyInput} bind:value={body} aria-label="Nachricht"></textarea>

  {#if error}<div class="error" role="alert">{error}</div>{/if}

  <footer>
    {#if confirmDiscard}
      <span class="discard-q">Entwurf verwerfen?</span>
      <button type="button" class="btn" onclick={() => (confirmDiscard = false)}>Weiter schreiben</button>
      <button type="button" class="btn danger" onclick={onclose}>Verwerfen</button>
    {:else}
      <span class="hint"><kbd>Strg</kbd>+<kbd>Enter</kbd> sendet</span>
      <button type="button" class="btn" onclick={close}>Abbrechen</button>
      <button type="button" class="btn primary" onclick={send} disabled={sending}>
        <Icon name="send" size={16} />
        {sending ? 'Wird gesendet …' : 'Senden'}
      </button>
    {/if}
  </footer>
</div>

<style>
  .composer {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    background: var(--surface-raised);
    border-left: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 18px 8px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 17px;
    font-weight: 650;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon-btn {
    border: 0;
    background: none;
    color: var(--text-muted);
    padding: 4px;
    border-radius: 6px;
  }
  .icon-btn:hover {
    background: var(--hover);
  }
  .fields {
    padding: 0 18px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    border-bottom: 1px solid var(--border);
    padding: 6px 0;
  }
  .row span {
    width: 56px;
    flex: none;
    color: var(--text-muted);
    font-size: 13px;
  }
  .row input,
  .row select {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    padding: 4px 0;
    color: var(--text);
  }
  .row select {
    background: var(--surface-raised);
  }
  .row:focus-within {
    border-bottom-color: var(--highlight);
  }
  .link-btn {
    border: 0;
    background: none;
    color: var(--accent-text);
    font-size: 13px;
    font-weight: 600;
  }
  textarea {
    flex: 1;
    margin: 10px 18px 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    font: 14px/1.55 var(--font);
    resize: none;
    outline: none;
  }
  textarea:focus {
    border-color: var(--highlight);
  }
  .error {
    margin: 8px 18px 0;
    padding: 6px 10px;
    border-radius: var(--radius);
    border: 1px solid color-mix(in srgb, var(--danger) 50%, transparent);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: 13px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
    padding: 12px 18px 16px;
  }
  .hint,
  .discard-q {
    margin-right: auto;
    font-size: 12px;
    color: var(--text-muted);
  }
  .discard-q {
    font-size: 13px;
    color: var(--text);
    font-weight: 600;
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-weight: 600;
  }
  .btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .btn.primary {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
    border-color: transparent;
  }
  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
    background: var(--accent-fill);
  }
  .btn.danger {
    background: var(--danger);
    color: #ffffff;
    border-color: transparent;
  }
  .btn:disabled {
    opacity: 0.7;
    cursor: progress;
  }
</style>

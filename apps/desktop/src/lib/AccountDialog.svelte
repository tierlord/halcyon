<script lang="ts">
  import { onMount } from 'svelte'
  import * as api from './api'
  import Icon from './Icon.svelte'
  import type {
    Account,
    DiscoverResult,
    DiscoverySource,
    Security,
    ServerConfig,
  } from './types'

  let {
    onclose,
    onchanged,
  }: {
    onclose: () => void
    onchanged: () => Promise<void> | void
  } = $props()

  type Mode = 'list' | 'edit'

  let mode: Mode = $state('list')
  let accounts: Account[] = $state([])
  let loadingList = $state(true)
  let confirmDeleteId: number | null = $state(null)

  // Editor state
  let editingId = $state(0)
  let email = $state('')
  let displayName = $state('')
  let password = $state('')
  let imap: ServerConfig = $state(defaultServer(993))
  let useSmtp = $state(false)
  let smtp: ServerConfig = $state(defaultServer(465))
  let showServerDetails = $state(false)

  // Discovery & connection check state
  let discovering = $state(false)
  let discoveryResult: DiscoverResult | null = $state(null)
  let selectedCandidate = $state(0)
  let lastDiscoveredEmail = $state('')
  let checking = $state(false)
  // Snapshot of the IMAP settings that passed the server check. Any edit to
  // the settings no longer matches it, which hides the password field again.
  let checkedServer: ServerConfig | null = $state(null)
  let saving = $state(false)
  let statusKind: 'idle' | 'ok' | 'warn' | 'err' = $state('idle')
  let statusText = $state('')

  const isNew = $derived(editingId === 0)
  const serverOk = $derived(checkedServer !== null && sameServer(checkedServer, effectiveImap()))
  let originalImap: ServerConfig | null = $state(null)
  const serverChanged = $derived(
    !isNew && originalImap !== null && !sameServer(originalImap, effectiveImap()),
  )
  const emailValid = $derived(/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()))

  function defaultServer(port: number): ServerConfig {
    return {
      host: '',
      port,
      security: port === 143 || port === 587 ? 'starttls' : 'tls',
      username: '',
      auth: 'password',
    }
  }

  /// The IMAP settings as they will be saved (empty username means address).
  function effectiveImap(): ServerConfig {
    return {
      ...imap,
      host: imap.host.trim(),
      port: Number(imap.port),
      username: imap.username.trim() || email.trim(),
    }
  }

  function sameServer(a: ServerConfig, b: ServerConfig): boolean {
    return (
      a.host.trim().toLowerCase() === b.host.trim().toLowerCase() &&
      Number(a.port) === Number(b.port) &&
      a.security === b.security &&
      (a.username.trim() || email.trim()) === (b.username.trim() || email.trim()) &&
      a.auth === b.auth
    )
  }

  // Mirrors ServerConfig::validate in the core, which enforces it.
  function isLocalHost(host: string): boolean {
    const h = host.trim().replace(/^\[|\]$/g, '').toLowerCase()
    return h === 'localhost' || h === '::1' || /^127\.\d+\.\d+\.\d+$/.test(h)
  }

  function sourceLabel(source: DiscoverySource): string {
    switch (source) {
      case 'provider':
        return 'Anbieter-Konfiguration'
      case 'ispdb':
        return 'Thunderbird-Datenbank'
      case 'mx-provider':
        return 'MX-Anbieter'
      case 'mx-ispdb':
        return 'MX-Datenbank'
      case 'srv':
        return 'DNS-SRV'
      case 'guess':
        return 'Geschätzt'
    }
  }

  async function loadAccounts() {
    loadingList = true
    try {
      accounts = await api.listAccounts()
    } catch (e) {
      statusKind = 'err'
      statusText = String(e)
    } finally {
      loadingList = false
    }
  }

  function startNewAccount() {
    mode = 'edit'
    editingId = 0
    email = ''
    displayName = ''
    password = ''
    imap = defaultServer(993)
    useSmtp = false
    smtp = defaultServer(465)
    showServerDetails = false
    discoveryResult = null
    selectedCandidate = 0
    lastDiscoveredEmail = ''
    checkedServer = null
    originalImap = null
    statusKind = 'idle'
    statusText = ''
  }

  function startEditAccount(acc: Account) {
    mode = 'edit'
    editingId = acc.id
    email = acc.email
    displayName = acc.display_name ?? ''
    password = ''
    imap = { ...acc.imap }
    // The account's current server is known to work.
    checkedServer = { ...acc.imap }
    originalImap = { ...acc.imap }
    useSmtp = acc.smtp !== null
    smtp = acc.smtp ? { ...acc.smtp } : defaultServer(465)
    showServerDetails = false
    discoveryResult = null
    selectedCandidate = 0
    lastDiscoveredEmail = acc.email
    statusKind = 'idle'
    statusText = ''
  }

  function applyCandidate(result: DiscoverResult, idx: number) {
    const c = result.candidates[idx]
    if (!c) return
    selectedCandidate = idx
    imap = { ...c.imap }
    if (c.smtp) {
      useSmtp = true
      smtp = { ...c.smtp }
    } else {
      useSmtp = false
      smtp = defaultServer(465)
    }
  }

  let discoverTimer: ReturnType<typeof setTimeout> | undefined
  let discoverSeq = 0

  function onEmailInput() {
    if (!isNew) return
    clearTimeout(discoverTimer)
    const trimmed = email.trim()
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(trimmed)) {
      discoveryResult = null
      statusKind = 'idle'
      statusText = ''
      return
    }
    if (trimmed.toLowerCase() === lastDiscoveredEmail.toLowerCase()) return
    discoverTimer = setTimeout(() => {
      runDiscovery(trimmed)
    }, 500)
  }

  async function runDiscovery(targetEmail = email.trim()) {
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(targetEmail)) return
    const seq = ++discoverSeq
    discovering = true
    lastDiscoveredEmail = targetEmail
    statusKind = 'idle'
    statusText = 'Suche Servereinstellungen und prüfe Verbindung …'

    try {
      const res = await api.discoverAccount(targetEmail)
      if (seq !== discoverSeq) return
      discoveryResult = res
      if (res.candidates.length === 0) {
        checkedServer = null
        showServerDetails = true
        statusKind = 'warn'
        statusText = 'Keine Einstellungen gefunden. Bitte Serverdaten manuell eingeben.'
        return
      }

      const bestIdx = res.reachable_index ?? 0
      applyCandidate(res, bestIdx)

      const chosen = res.candidates[bestIdx]
      const src = chosen.provider_name
        ? `${chosen.provider_name} (${sourceLabel(chosen.source)})`
        : sourceLabel(chosen.source)

      if (res.reachable_index !== null) {
        checkedServer = effectiveImap()
        statusKind = 'ok'
        statusText = `Server gefunden: ${chosen.imap.host} (${src}).`
      } else {
        checkedServer = null
        showServerDetails = true
        statusKind = 'warn'
        statusText = `Der Server ${chosen.imap.host} antwortet nicht (${res.probe_error ?? 'unbekannter Fehler'}). Bitte die Servereinstellungen prüfen.`
      }
    } catch (e) {
      if (seq !== discoverSeq) return
      showServerDetails = true
      statusKind = 'err'
      statusText = `Ermittlung fehlgeschlagen: ${String(e)}`
    } finally {
      if (seq === discoverSeq) discovering = false
    }
  }

  function onImapSecurityChange(sec: Security) {
    imap.security = sec
    if (sec === 'tls' && imap.port === 143) imap.port = 993
    if (sec !== 'tls' && imap.port === 993) imap.port = 143
  }

  function onSmtpSecurityChange(sec: Security) {
    smtp.security = sec
    if (sec === 'tls' && smtp.port === 587) smtp.port = 465
    if (sec !== 'tls' && smtp.port === 465) smtp.port = 587
  }

  // A success message must not outlive the settings it was about.
  $effect(() => {
    if (!serverOk && statusKind === 'ok') {
      statusKind = 'idle'
      statusText = ''
    }
  })

  async function checkServer() {
    checking = true
    statusKind = 'idle'
    statusText = 'Prüfe den Server …'
    const target = effectiveImap()
    try {
      const msg = await api.checkServer(email.trim(), target)
      checkedServer = target
      statusKind = 'ok'
      statusText = msg
    } catch (e) {
      checkedServer = null
      statusKind = 'err'
      statusText = String(e)
    } finally {
      checking = false
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    if (!emailValid || !serverOk) return
    if (password === '' && (isNew || serverChanged)) return
    saving = true
    statusKind = 'idle'
    statusText = password ? 'Prüfe Anmeldung und speichere Konto …' : 'Speichere Konto …'

    const payload: Account = {
      id: editingId,
      email: email.trim(),
      display_name: displayName.trim() || null,
      imap: effectiveImap(),
      smtp:
        useSmtp && smtp.host.trim()
          ? {
              ...smtp,
              host: smtp.host.trim(),
              port: Number(smtp.port),
              username: smtp.username.trim() || email.trim(),
            }
          : null,
    }

    try {
      await api.saveAccount(payload, password || null)
      await loadAccounts()
      await onchanged()
      mode = 'list'
      statusKind = 'idle'
      statusText = ''
    } catch (err) {
      statusKind = 'err'
      statusText = String(err)
    } finally {
      saving = false
    }
  }

  async function removeAccount(id: number) {
    try {
      await api.deleteAccount(id)
      confirmDeleteId = null
      await loadAccounts()
      await onchanged()
    } catch (e) {
      statusKind = 'err'
      statusText = String(e)
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation()
      onclose()
    }
  }

  onMount(() => {
    loadAccounts()
  })
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="account-dialog-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
  >
    <header class="dialog-header">
      <h2 id="account-dialog-title">
        {#if mode === 'list'}
          Konten verwalten
        {:else if isNew}
          Neues Konto einrichten
        {:else}
          Konto bearbeiten
        {/if}
      </h2>
      <div class="header-actions">
        {#if mode === 'list'}
          <button type="button" class="btn primary" onclick={startNewAccount}>
            <Icon name="plus" size={16} />
            <span>Neues Konto</span>
          </button>
        {/if}
        <button type="button" class="icon-btn" onclick={onclose} aria-label="Dialog schließen">
          <Icon name="x" size={18} />
        </button>
      </div>
    </header>

    {#if mode === 'list'}
      <div class="dialog-body">
        {#if statusText && statusKind === 'err'}
          <div class="banner err" role="alert">{statusText}</div>
        {/if}

        {#if loadingList}
          <p class="muted">Lade Konten …</p>
        {:else if accounts.length === 0}
          <div class="empty-box">
            <p>Noch keine E-Mail-Konten eingerichtet.</p>
            <button type="button" class="btn primary" onclick={startNewAccount}>
              <Icon name="plus" size={16} />
              <span>Erstes Konto hinzufügen</span>
            </button>
          </div>
        {:else}
          <ul class="account-list">
            {#each accounts as acc (acc.id)}
              <li class="account-row">
                <div class="account-meta">
                  <div class="account-title">
                    {acc.display_name ? `${acc.display_name} · ` : ''}{acc.email}
                  </div>
                  <div class="account-sub">
                    IMAP: {acc.imap.host}:{acc.imap.port} ({acc.imap.security.toUpperCase()})
                    {#if acc.smtp}
                      · SMTP: {acc.smtp.host}:{acc.smtp.port}
                    {/if}
                  </div>
                </div>
                <div class="row-actions">
                  {#if confirmDeleteId === acc.id}
                    <span class="confirm-text">Entfernen?</span>
                    <button
                      type="button"
                      class="btn danger"
                      onclick={() => removeAccount(acc.id)}
                    >
                      Ja
                    </button>
                    <button
                      type="button"
                      class="btn"
                      onclick={() => (confirmDeleteId = null)}
                    >
                      Nein
                    </button>
                  {:else}
                    <button
                      type="button"
                      class="btn"
                      onclick={() => startEditAccount(acc)}
                      title="Konto bearbeiten"
                    >
                      <Icon name="pencil" size={15} />
                      <span>Bearbeiten</span>
                    </button>
                    <button
                      type="button"
                      class="icon-btn danger-icon"
                      onclick={() => (confirmDeleteId = acc.id)}
                      title="Konto entfernen"
                      aria-label="Konto entfernen"
                    >
                      <Icon name="trash" size={16} />
                    </button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {:else}
      <form class="dialog-body form" onsubmit={submit}>
        <label class="field">
          <span>E-Mail-Adresse</span>
          <div class="input-row">
            <input
              type="email"
              bind:value={email}
              oninput={onEmailInput}
              placeholder="name@example.org"
              required
              disabled={!isNew}
              autocomplete="email"
            />
            {#if isNew}
              <button
                type="button"
                class="btn"
                disabled={!emailValid || discovering}
                onclick={() => runDiscovery()}
              >
                {discovering ? 'Suche …' : 'Prüfen'}
              </button>
            {/if}
          </div>
        </label>

        {#if statusText}
          <div
            class="banner"
            class:ok={statusKind === 'ok'}
            class:warn={statusKind === 'warn'}
            class:err={statusKind === 'err'}
            role={statusKind === 'err' ? 'alert' : 'status'}
          >
            {#if statusKind === 'ok'}<Icon name="check" size={16} />{/if}
            <span>{statusText}</span>
          </div>
        {/if}


        <label class="field">
          <span>Anzeigename <small>(optional)</small></span>
          <input
            type="text"
            bind:value={displayName}
            placeholder="z. B. Privat oder Arbeit"
          />
        </label>

        {#if discoveryResult && discoveryResult.candidates.length > 1}
          <label class="field">
            <span>Gefundene Serverkonfiguration</span>
            <select
              value={selectedCandidate}
              onchange={(e) => {
                if (discoveryResult) {
                  applyCandidate(discoveryResult, Number(e.currentTarget.value))
                }
              }}
            >
              {#each discoveryResult.candidates as cand, idx}
                <option value={idx}>
                  {cand.imap.host}:{cand.imap.port} ({cand.imap.security.toUpperCase()}) – {sourceLabel(
                    cand.source,
                  )}
                </option>
              {/each}
            </select>
          </label>
        {/if}

        <div class="details-toggle">
          <button
            type="button"
            class="link-btn"
            onclick={() => (showServerDetails = !showServerDetails)}
          >
            {showServerDetails ? '▾ Servereinstellungen ausblenden' : '▸ Servereinstellungen anpassen'}
          </button>
          {#if !showServerDetails && imap.host}
            <span class="muted small">
              {imap.host}:{imap.port} ({imap.security.toUpperCase()})
            </span>
          {/if}
        </div>

        {#if showServerDetails}
          <fieldset class="server-box">
            <legend>Posteingang (IMAP)</legend>
            <div class="grid-2">
              <label class="field">
                <span>Server</span>
                <input type="text" bind:value={imap.host} placeholder="imap.example.org" required />
              </label>
              <label class="field">
                <span>Port</span>
                <input type="number" min="1" max="65535" bind:value={imap.port} required />
              </label>
              <label class="field">
                <span>Sicherheit</span>
                <select
                  value={imap.security}
                  onchange={(e) => onImapSecurityChange(e.currentTarget.value as Security)}
                >
                  <option value="tls">SSL/TLS</option>
                  <option value="starttls">STARTTLS</option>
                  {#if isLocalHost(imap.host)}
                    <option value="plain">Unverschlüsselt (nur lokal)</option>
                  {/if}
                </select>
              </label>
              <label class="field">
                <span>Benutzername</span>
                <input
                  type="text"
                  bind:value={imap.username}
                  placeholder={email || 'name@example.org'}
                />
              </label>
            </div>
          </fieldset>

          <label class="checkbox">
            <input type="checkbox" bind:checked={useSmtp} />
            <span>Postausgangsserver (SMTP) konfigurieren</span>
          </label>

          {#if useSmtp}
            <fieldset class="server-box">
              <legend>Postausgang (SMTP)</legend>
              <div class="grid-2">
                <label class="field">
                  <span>Server</span>
                  <input type="text" bind:value={smtp.host} placeholder="smtp.example.org" />
                </label>
                <label class="field">
                  <span>Port</span>
                  <input type="number" min="1" max="65535" bind:value={smtp.port} />
                </label>
                <label class="field">
                  <span>Sicherheit</span>
                  <select
                    value={smtp.security}
                    onchange={(e) => onSmtpSecurityChange(e.currentTarget.value as Security)}
                  >
                    <option value="tls">SSL/TLS</option>
                    <option value="starttls">STARTTLS</option>
                    {#if isLocalHost(smtp.host)}
                      <option value="plain">Unverschlüsselt (nur lokal)</option>
                    {/if}
                  </select>
                </label>
                <label class="field">
                  <span>Benutzername</span>
                  <input
                    type="text"
                    bind:value={smtp.username}
                    placeholder={email || 'name@example.org'}
                  />
                </label>
              </div>
            </fieldset>
          {/if}
        {/if}

        {#if serverOk}
          <div class="password-step">
            <label class="field">
              <span>
                Passwort für <strong>{effectiveImap().username}</strong> auf
                <strong>{effectiveImap().host}</strong>
              </span>
              <input
                type="password"
                bind:value={password}
                placeholder={isNew || serverChanged
                  ? 'Passwort oder App-Passwort'
                  : 'Leer lassen, um das gespeicherte Passwort zu behalten'}
                required={isNew || serverChanged}
                autocomplete="current-password"
              />
            </label>
          </div>
        {:else if emailValid && imap.host.trim() && !discovering}
          <p class="muted small step-hint">
            Das Passwort wird abgefragt, sobald der Server geprüft ist.
          </p>
        {/if}

        <footer class="form-footer">
          <button type="button" class="btn" onclick={() => (mode = 'list')} disabled={saving}>
            Zurück
          </button>
          <div class="spacer"></div>
          {#if !serverOk}
            <button
              type="button"
              class="btn primary"
              onclick={checkServer}
              disabled={!emailValid || !imap.host.trim() || checking || discovering}
            >
              {checking ? 'Prüfe …' : 'Server prüfen'}
            </button>
          {/if}
{#if serverOk}
          <button
            type="submit"
            class="btn primary"
            disabled={!emailValid ||
              !serverOk ||
              saving ||
              discovering ||
              (password === '' && (isNew || serverChanged))}
          >
            {#if saving}
              Melde an …
            {:else if isNew}
              Konto hinzufügen
            {:else}
              Speichern
            {/if}
          </button>
          {/if}
        </footer>
      </form>
    {/if}
  </div>
</div>

<style>
  .password-step {
    padding: 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--sky-aqua) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--sky-aqua) 40%, transparent);
  }
  .password-step strong {
    font-weight: 650;
  }
  .step-hint {
    margin: 0;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--pitch-black) 55%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    z-index: 100;
  }
  .dialog {
    width: min(560px, 100%);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: calc(var(--radius) + 4px);
    box-shadow: 0 18px 48px color-mix(in srgb, var(--pitch-black) 35%, transparent);
    overflow: hidden;
  }
  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .dialog-header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 650;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dialog-body {
    padding: 16px 18px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .account-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .account-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .account-meta {
    min-width: 0;
  }
  .account-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .account-sub {
    font-size: 12px;
    color: var(--text-muted);
  }
  .row-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }
  .confirm-text {
    font-size: 12px;
    color: var(--danger);
    font-weight: 600;
  }
  .empty-box {
    text-align: center;
    padding: 24px 12px;
    color: var(--text-muted);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .field > span {
    font-weight: 600;
  }
  .field small {
    font-weight: 400;
    color: var(--text-muted);
  }
  .input-row {
    display: flex;
    gap: 8px;
  }
  .input-row input {
    flex: 1;
    min-width: 0;
  }
  input[type='email'],
  input[type='text'],
  input[type='password'],
  input[type='number'],
  select {
    width: 100%;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  input:disabled {
    opacity: 0.7;
    background: var(--surface);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 13px;
  }
  .banner.ok {
    border-color: color-mix(in srgb, var(--accent-fill) 55%, transparent);
    background: color-mix(in srgb, var(--accent-fill) 12%, transparent);
    color: var(--text);
  }
  .banner.ok :global(svg) {
    color: var(--accent-text);
    flex: none;
  }
  .banner.warn {
    border-color: color-mix(in srgb, var(--tangerine-dream) 65%, transparent);
    background: color-mix(in srgb, var(--tangerine-dream) 16%, transparent);
  }
  .banner.err {
    border-color: color-mix(in srgb, var(--danger) 55%, transparent);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .details-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .link-btn {
    border: 0;
    background: none;
    padding: 2px 0;
    color: var(--accent-text);
    font-weight: 600;
    font-size: 13px;
  }
  .server-box {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px 12px 12px;
    margin: 0;
  }
  .server-box legend {
    padding: 0 6px;
    font-size: 12px;
    font-weight: 650;
    color: var(--text-muted);
  }
  .grid-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
  }
  .form-footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    font-size: 13px;
  }
  .btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .btn.primary {
    border-color: transparent;
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn.danger {
    border-color: transparent;
    background: var(--danger);
    color: var(--white);
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    color: var(--text-muted);
  }
  .icon-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .danger-icon:hover {
    color: var(--danger);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 12px;
  }
</style>

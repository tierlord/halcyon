<script lang="ts" module>
  /** Where a "Wohin gehört das?" request comes from. */
  export interface AssignRequest {
    messageId: number
    /** The chosen target; `null` asks for the name of a new bundle first. */
    target: AssignChoice
    /** Where the mail is now. */
    current: Target
    /** The card, row or tile (`[data-assignable]`). */
    sourceEl: HTMLElement | null
    /** The menu button, if the menu was used; naming and focus belong to it. */
    button?: HTMLElement | null
    name: string
    brandEmail: string | null
    style: string
  }
</script>

<script lang="ts">
  // Everything around "Wohin gehört das?" that lives above the page: the drag
  // ghost, the dock with the drop stacks, the naming prompt and the preview
  // scene. Today.svelte only marks cards, rows and tiles; it calls `start`.
  import { onMount } from 'svelte'
  import * as api from './api'
  import AssignScene, { type SceneSource } from './AssignScene.svelte'
  import BrandAvatar from './BrandAvatar.svelte'
  import type { BundleStore } from './bundleStore.svelte'
  import NamePrompt from './NamePrompt.svelte'
  import {
    FOR_YOU,
    bundleTarget,
    orderedBundles,
    sameTarget,
    targetKey,
    targetName,
    type AssignmentPreview,
    type Target,
  } from './bundles'
  import {
    DELETE,
    GHOST_DX,
    GHOST_DY,
    drag,
    dropTarget,
    onAssignDrop,
    registerGhost,
    type AssignChoice,
    type DragSpec,
  } from './assignDrag.svelte'
  import Icon from './Icon.svelte'
  import type { MessageSummary } from './types'

  let {
    store,
    onofferundo,
    ondelete,
  }: {
    store: BundleStore
    /** Shows the undo bar. */
    onofferundo: (label: string, undo: () => void) => void
    /** Moves a mail to the trash (with its own undo bar); the parent owns that. */
    ondelete: (ids: number[], label: string) => void
  } = $props()

  interface Scene {
    id: number
    req: AssignRequest
    target: Target
    source: SceneSource
    preview: AssignmentPreview | null
    error: string | null
  }

  let scene = $state<Scene | null>(null)
  let naming = $state<{ req: AssignRequest; anchor: HTMLElement | null } | null>(null)
  let ghostEl: HTMLElement | undefined = $state()
  let nextScene = 1

  const dockTargets = $derived<{ target: Target; name: string }[]>([
    { target: FOR_YOU, name: 'Für dich' },
    ...orderedBundles(store.bundles)
      .filter((b) => b.enabled)
      .map((b) => ({ target: bundleTarget(b.key), name: b.name })),
  ])

  const quoted = (name: string) => `‚${name}‘`

  /** Entry point for the menu, for drops and for "Neues Bündel …". */
  export async function start(req: AssignRequest) {
    if (scene) return
    if (req.target == null) {
      naming = { req, anchor: req.button ?? req.sourceEl }
      return
    }
    if (req.target.kind === 'delete') {
      // No preview scene: nothing here is "all of this kind". The undo bar
      // takes over; focus moves on when the source disappears.
      ondelete([req.messageId], 'Gelöscht')
      return
    }
    await open(req, req.target)
  }

  async function open(req: AssignRequest, target: Target) {
    if (sameTarget(target, req.current)) return
    const el = req.sourceEl
    // Room for the scene next to the source.
    el?.scrollIntoView({ block: 'center', behavior: 'auto' })
    const rect = el?.getBoundingClientRect() ?? new DOMRect(window.innerWidth / 2 - 150, 120, 300, 60)
    const source: SceneSource = {
      el,
      rect,
      name: req.name,
      brandEmail: req.brandEmail,
      style: req.style,
      focusEl: req.button ?? el,
    }
    const id = nextScene++
    scene = { id, req, target, source, preview: null, error: null }
    try {
      const preview = await api.assignmentPreview(req.messageId, target)
      if (scene?.id === id) scene.preview = preview
    } catch (e) {
      if (scene?.id === id) scene.error = String(e)
    }
  }

  async function createAndOpen(name: string) {
    const pending = naming
    if (!pending) return
    const bundle = await store.create(name)
    if (!bundle) return // the prompt shows store.error and stays open
    naming = null
    await open(pending.req, bundleTarget(bundle.key))
  }

  async function findMails(ids: number[]): Promise<MessageSummary[]> {
    return api.messagesByIds(ids)
  }

  function flyTarget(): DOMRect | null {
    if (!scene) return null
    const el = document.querySelector<HTMLElement>(`[data-drop="${targetKey(scene.target)}"]:not([data-dock])`)
    return el?.getBoundingClientRect() ?? null
  }

  function labelFor(t: Target): string {
    return t.kind === 'for_you' ? 'Unter „Für dich“ eingeordnet' : `In ${quoted(targetName(t, store.bundles))} eingeordnet`
  }

  async function apply(kindKeys: string[], excluded: number[], alsoThis: boolean) {
    if (!scene) return
    const { req, target } = scene
    const ids: number[] = []
    if (alsoThis) ids.push(...(await store.apply(req.messageId, target, [], [])))
    ids.push(...(await store.apply(req.messageId, target, kindKeys, excluded)))
    onofferundo(labelFor(target), () => void store.undo(ids))
  }

  function closed(applied: boolean) {
    const focus = scene?.source.focusEl
    scene = null
    if (!applied && focus?.isConnected) focus.focus()
  }

  onMount(() =>
    onAssignDrop((spec: DragSpec, target, source) =>
      void start({
        messageId: spec.messageId,
        target,
        current: spec.current,
        sourceEl: source,
        name: spec.name,
        brandEmail: spec.brandEmail,
        style: spec.style,
      }),
    ),
  )

  $effect(() => {
    registerGhost(ghostEl ?? null)
    return () => registerGhost(null)
  })
</script>

{#if drag.active && drag.spec}
  <div
    bind:this={ghostEl}
    class="ghost brand-scope"
    style={drag.spec.style}
    style:transform={`translate(${drag.x - GHOST_DX}px, ${drag.y - GHOST_DY}px)`}
    aria-hidden="true"
  >
    <BrandAvatar email={drag.spec.brandEmail} name={drag.spec.name} size={32} />
    <span class="g-label">{drag.spec.label}</span>
  </div>

  {#if !drag.returning}
    <!-- The way to every place at once, within reach of the pointer. -->
    <div class="dock" aria-hidden="true">
      {#each dockTargets as t (targetKey(t.target))}
        <div
          class="dstack"
          class:here={sameTarget(t.target, drag.spec.current)}
          use:dropTarget={t.target}
          data-dock
        >
          <span class="d-back d2"></span>
          <span class="d-back d1"></span>
          <span class="d-front">{t.name}</span>
          {#if sameTarget(t.target, drag.spec.current)}<span class="d-note">liegt hier</span>{/if}
        </div>
      {/each}
      <!-- Apart from the places: the bin ends the row. -->
      <div class="dstack bin" use:dropTarget={DELETE} data-dock>
        <span class="d-front"><Icon name="trash" size={18} /> Löschen</span>
      </div>
    </div>
  {/if}
{/if}

{#if naming}
  <NamePrompt
    title="Neues Bündel"
    confirm="Anlegen"
    anchor={naming.anchor}
    error={store.error}
    onsubmit={createAndOpen}
    oncancel={() => (naming = null)}
  />
{/if}

{#if scene}
  {#key scene.id}
    <AssignScene
      source={scene.source}
      messageId={scene.req.messageId}
      heading={scene.target.kind === 'for_you'
        ? 'Unter „Für dich“ zeigen'
        : `In ${quoted(targetName(scene.target, store.bundles))} einordnen`}
      preview={scene.preview}
      error={scene.error}
      {findMails}
      {flyTarget}
      onapply={apply}
      onclose={closed}
    />
  {/key}
{/if}

<style>
  :global(body[data-assign-drag], body[data-assign-drag] *) {
    cursor: grabbing !important;
  }
  :global([data-dragging='true']) {
    opacity: 0.45;
  }
  /* While dragging, every place a mail can go shows itself; the one under the
     pointer lights up. Outlines only: nothing may change size under the pointer. */
  :global(body[data-assign-drag] [data-drop]:not([data-dock])) {
    outline: 1.5px dashed color-mix(in srgb, var(--highlight) 70%, transparent);
    outline-offset: 3px;
  }
  :global([data-drop][data-drop-hot='true']:not([data-dock])) {
    outline: 2.5px solid var(--highlight) !important;
    outline-offset: 3px;
    background-color: color-mix(in srgb, var(--highlight) 10%, transparent);
  }

  .ghost {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 75;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 280px;
    padding: 8px 14px 8px 8px;
    border-radius: 16px;
    background: linear-gradient(160deg, var(--tint), var(--tint-end) 80%), var(--surface-raised);
    box-shadow:
      0 14px 34px -8px color-mix(in srgb, var(--pitch-black) 45%, transparent),
      0 0 0 1.5px var(--brand-acc, var(--highlight));
    pointer-events: none;
    will-change: transform;
  }
  .g-label {
    min-width: 0;
    font-size: 13.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dock {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 30;
    display: flex;
    justify-content: center;
    gap: 14px;
    padding: 34px 24px 16px;
    overflow-x: auto;
    background: linear-gradient(transparent, color-mix(in srgb, var(--bg) 92%, transparent) 55%);
    animation: dock-in 0.22s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes dock-in {
    from {
      transform: translateY(100%);
      opacity: 0;
    }
  }
  .dstack {
    position: relative;
    flex: none;
    width: 112px;
    height: 64px;
  }
  .d-front,
  .d-back {
    position: absolute;
    inset: 0;
    border-radius: 14px;
    background: var(--card-surface);
    box-shadow: var(--card-shadow), var(--card-highlight), 0 0 0 1px color-mix(in srgb, var(--text) 10%, transparent);
  }
  .d-back.d1 {
    transform: translate(3px, -4px) rotate(1.5deg);
    opacity: 0.8;
  }
  .d-back.d2 {
    transform: translate(-3px, -8px) rotate(-2deg);
    opacity: 0.55;
  }
  .d-front {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px 8px;
    text-align: center;
    font-size: 12.5px;
    font-weight: 600;
    line-height: 1.25;
  }
  .d-note {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -15px;
    text-align: center;
    font-size: 11px;
    color: var(--text-soft);
  }
  .dstack.bin {
    margin-left: 18px;
  }
  .dstack.bin .d-front {
    flex-direction: column;
    gap: 4px;
    color: var(--text-soft);
  }
  .dstack.bin:global([data-drop-hot='true']) .d-front {
    background: color-mix(in srgb, var(--danger) 18%, var(--card-surface));
    box-shadow: 0 0 0 2.5px var(--danger);
    color: var(--danger);
  }
  .dstack.here {
    opacity: 0.55;
  }
  .dstack:global([data-drop-hot='true']) .d-front {
    background: color-mix(in srgb, var(--highlight) 22%, var(--card-surface));
    box-shadow: 0 0 0 2.5px var(--highlight);
  }
  @media (prefers-reduced-motion: reduce) {
    .dock {
      animation: none;
    }
  }
</style>

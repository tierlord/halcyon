// Drag and drop for "Wohin gehört das?", built on pointer events (WebKitGTK,
// the Linux target, is unreliable with the HTML5 drag API).
//
// Dragging starts only from a handle (`[data-drag-handle]`: the logo tile or a
// small grip), so text everywhere else stays selectable. Only a press on the
// handle blocks selection.
//
// The element under the pointer never moves (design/interaktion.md): dragging
// lifts a small ghost that follows the pointer while the original stays where
// it is, dimmed. Drop targets are plain elements marked with `data-drop`;
// the layer (AssignLayer.svelte) renders the ghost and the dock and receives
// the drop. Every gesture has an equivalent in the menu (AssignMenu.svelte).

import { parseTargetKey, sameTarget, targetKey, type Target } from './bundles'

/** What is being dragged, as far as the ghost and the drop handler need it. */
export interface DragSpec {
  messageId: number
  /** Where the mail is now; dropping it there does nothing. */
  current: Target
  /** Text on the ghost. */
  label: string
  brandEmail: string | null
  name: string
  /** Inline `--brand*` properties of the source (see brandColor.ts). */
  style: string
}

/** Reactive part: what the layer draws. */
export const drag = $state({
  active: false,
  spec: null as DragSpec | null,
  /** Pointer position (viewport pixels). */
  x: 0,
  y: 0,
  /** `targetKey` of the drop target under the pointer. */
  over: null as string | null,
  /** The ghost is flying back after a cancelled drag. */
  returning: false,
})

/** Offset of the ghost's top left from the pointer. */
export const GHOST_DX = 24
export const GHOST_DY = 20

const THRESHOLD = 6

/** Drop key of the "Löschen" target; it is not a bundle, so it has no `Target`. */
export const DELETE_KEY = 'delete'
/** A drop on the delete target. */
export const DELETE = { kind: 'delete' } as const
export type DropTarget = Target | typeof DELETE
/** What the menu or a drop hands over: a place, the bin, or `null` for "Neues Bündel …". */
export type AssignChoice = DropTarget | null

export const HANDLE_HINT = 'Ziehen, um einzuordnen'

interface Session {
  node: HTMLElement
  spec: DragSpec
  sx: number
  sy: number
  moved: boolean
}

let session: Session | null = null
let hotEl: HTMLElement | null = null
let raf = 0
let ghostEl: HTMLElement | null = null
let dropHandler: ((spec: DragSpec, target: DropTarget, source: HTMLElement) => void) | null = null

const reduceMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches

export function registerGhost(el: HTMLElement | null) {
  ghostEl = el
}

/** The layer takes the drop; returns the function that unregisters it. */
export function onAssignDrop(fn: (spec: DragSpec, target: DropTarget, source: HTMLElement) => void) {
  dropHandler = fn
  return () => {
    if (dropHandler === fn) dropHandler = null
  }
}

/**
 * Makes an element draggable onto a bundle, "Für dich" or the bin, from its
 * handle (`[data-drag-handle]` inside it). `null` switches it off.
 */
export function assignable(node: HTMLElement, param: DragSpec | null) {
  let spec = param
  const down = (e: PointerEvent) => {
    // Touch keeps scrolling; it uses the menu.
    if (!spec || e.button !== 0 || e.pointerType === 'touch' || session) return
    const handle = (e.target as Element).closest('[data-drag-handle]')
    if (!handle || !node.contains(handle)) return
    session = { node, spec, sx: e.clientX, sy: e.clientY, moved: false }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', abort)
    window.addEventListener('keydown', key, true)
    // WebKit starts the text selection on press, before the drag threshold
    // is reached; setting user-select afterwards does not stop a selection
    // that is already under way, so it is refused from the start (handle
    // presses only: everything else stays selectable).
    window.addEventListener('selectstart', noSelect, true)
    window.addEventListener('dragstart', noSelect, true)
  }
  node.addEventListener('pointerdown', down)
  return {
    update(next: DragSpec | null) {
      spec = next
    },
    destroy() {
      node.removeEventListener('pointerdown', down)
      if (session?.node === node) end()
    },
  }
}

/** Marks an element as a place a dragged mail can be dropped. */
export function dropTarget(node: HTMLElement, target: DropTarget | null) {
  const set = (t: DropTarget | null) => {
    if (t) node.setAttribute('data-drop', t.kind === 'delete' ? DELETE_KEY : targetKey(t))
    else node.removeAttribute('data-drop')
  }
  set(target)
  return {
    update: set,
    destroy() {
      node.removeAttribute('data-drop')
      if (hotEl === node) hotEl = null
    },
  }
}

function move(e: PointerEvent) {
  if (!session) return
  if (!session.moved) {
    if (Math.hypot(e.clientX - session.sx, e.clientY - session.sy) < THRESHOLD) return
    session.moved = true
    drag.active = true
    drag.returning = false
    drag.spec = session.spec
    session.node.setAttribute('data-dragging', 'true')
    document.body.setAttribute('data-assign-drag', '')
    setUserSelect('none')
    getSelection()?.removeAllRanges()
    raf = requestAnimationFrame(autoscroll)
  }
  drag.x = e.clientX
  drag.y = e.clientY
  hitTest()
}

function hitTest() {
  let found: HTMLElement | null = null
  for (const el of document.elementsFromPoint(drag.x, drag.y)) {
    const t = el.closest<HTMLElement>('[data-drop]')
    if (t) {
      found = t
      break
    }
  }
  if (found !== hotEl) {
    hotEl?.removeAttribute('data-drop-hot')
    found?.setAttribute('data-drop-hot', 'true')
    hotEl = found
  }
  drag.over = found?.getAttribute('data-drop') ?? null
}

/** Near the top or bottom edge of the view the page scrolls along. */
function autoscroll() {
  if (!drag.active || drag.returning) return
  const scroller = session?.node.closest<HTMLElement>('.today')
  if (scroller && !drag.over) {
    const r = scroller.getBoundingClientRect()
    const edge = 64
    let dy = 0
    if (drag.y < r.top + edge) dy = -Math.ceil((r.top + edge - drag.y) / 6)
    else if (drag.y > r.bottom - edge) dy = Math.ceil((drag.y - (r.bottom - edge)) / 6)
    if (dy) {
      scroller.scrollTop += dy
      hitTest()
    }
  }
  raf = requestAnimationFrame(autoscroll)
}

function up() {
  if (!session) return
  if (!session.moved) {
    end()
    return
  }
  suppressClick()
  const { node, spec } = session
  if (drag.over === DELETE_KEY) {
    end()
    dropHandler?.(spec, DELETE, node)
    return
  }
  if (drag.over) {
    const target = parseTargetKey(drag.over)
    if (!sameTarget(target, spec.current)) {
      end()
      dropHandler?.(spec, target, node)
      return
    }
  }
  flyBack()
}

function abort() {
  if (session?.moved) flyBack()
  else end()
}

function key(e: KeyboardEvent) {
  if (e.key !== 'Escape' || !session) return
  e.preventDefault()
  e.stopPropagation()
  abort()
}

/** The ghost glides back to where it came from. */
function flyBack() {
  if (!session) return
  stopListening()
  cancelAnimationFrame(raf)
  hotEl?.removeAttribute('data-drop-hot')
  hotEl = null
  drag.over = null
  const rect = session.node.getBoundingClientRect()
  if (!ghostEl || reduceMotion()) {
    end()
    return
  }
  drag.returning = true
  const from = `translate(${drag.x - GHOST_DX}px, ${drag.y - GHOST_DY}px)`
  const to = `translate(${rect.left}px, ${rect.top}px)`
  const anim = ghostEl.animate(
    [
      { transform: from, opacity: 1 },
      { transform: to, opacity: 0.4 },
    ],
    { duration: 220, easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)', fill: 'forwards' },
  )
  anim.onfinish = end
  anim.oncancel = end
}

function stopListening() {
  window.removeEventListener('pointermove', move)
  window.removeEventListener('pointerup', up)
  window.removeEventListener('pointercancel', abort)
  window.removeEventListener('keydown', key, true)
  window.removeEventListener('selectstart', noSelect, true)
  window.removeEventListener('dragstart', noSelect, true)
}

function noSelect(e: Event) {
  e.preventDefault()
}

/** WebKitGTK only knows the prefixed property. */
export function setUserSelect(value: string) {
  const style = document.body.style as CSSStyleDeclaration & { webkitUserSelect?: string }
  style.userSelect = value
  style.webkitUserSelect = value
}

function end() {
  stopListening()
  cancelAnimationFrame(raf)
  session?.node.removeAttribute('data-dragging')
  hotEl?.removeAttribute('data-drop-hot')
  hotEl = null
  document.body.removeAttribute('data-assign-drag')
  setUserSelect('')
  drag.active = false
  drag.returning = false
  drag.over = null
  drag.spec = null
  session = null
}

/** A drag ends with a click on the element; that must not open the mail. */
function suppressClick() {
  const stop = (ev: Event) => {
    ev.stopPropagation()
    ev.preventDefault()
  }
  window.addEventListener('click', stop, { capture: true, once: true })
  setTimeout(() => window.removeEventListener('click', stop, true), 60)
}

/**
 * Text the user has selected inside `el`. A click that ends a text selection
 * must not trigger the click action (open, expand) of the element around it.
 */
export function selectedTextIn(el: Element | null): boolean {
  const sel = getSelection()
  if (!el || !sel || sel.isCollapsed || !sel.anchorNode) return false
  return el.contains(sel.anchorNode)
}

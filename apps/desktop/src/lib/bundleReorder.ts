// Dragging a bundle by the handle of its header line to a new place in the
// order. Pointer events only (WebKitGTK is unreliable with the HTML5 drag API).
//
// Kept apart from the mail drag in assignDrag.svelte.ts: it uses its own
// handle attribute (`data-bundle-handle`), so a bundle drag can never start a
// mail assignment, and it sets no `data-assign-drag`, so no drop targets light up.
//
// Layout contract with BundleArea.svelte: every bundle sits in a
// `[data-bundle-item="<key>"]` slot that is a direct child of
// `[data-bundle-list]`; `other` is the last slot and never moves.
//
// The element under the pointer never moves (design/interaktion.md): the
// pointer drags a ghost of the header; the original slot stays in the flow and
// only the *other* slots slide to open the gap. Where the gap goes is decided
// from the original positions measured when the drag starts, not from the
// moving slots, so nothing feeds back into the decision. Every move has an
// equivalent in the bundle's menu ("Nach oben" / "Nach unten").

import { setUserSelect } from './assignDrag.svelte'
import { OTHER_KEY } from './bundles'

export const BUNDLE_HANDLE_HINT = 'Ziehen, um das Bündel zu verschieben'

const THRESHOLD = 6
const EDGE = 64

interface Slot {
  el: HTMLElement
  key: string
  /** Top in scroller content coordinates, measured at the start. */
  top: number
  height: number
  /** Space the slot takes in the flow: from its top to the next one's. */
  size: number
}

export interface ReorderParam {
  /** Called once on drop when the order changed; the store saves it. */
  onreorder: (anchorKey: string, side: 'before' | 'after') => void
}

interface Session {
  node: HTMLElement
  param: ReorderParam
  sx: number
  sy: number
  px: number
  py: number
  moved: boolean
  list: HTMLElement
  scroller: HTMLElement | null
  scroll0: number
  self: Slot
  movable: Slot[]
  fromIndex: number
  index: number
  ghost: HTMLElement | null
  /** Header offset inside its slot, and its size and left edge. */
  offset: number
  headerH: number
  left: number
  top0: number
  gx: number
  gy: number
  raf: number
}

let session: Session | null = null

const reduceMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches

/** Put on the header element that holds the `[data-bundle-handle]` grip. */
export function reorderable(node: HTMLElement, initial: ReorderParam) {
  let param = initial
  const down = (e: PointerEvent) => {
    if (e.button !== 0 || e.pointerType === 'touch' || session) return
    // Never while a mail is being dragged.
    if (document.body.hasAttribute('data-assign-drag')) return
    const handle = (e.target as Element).closest('[data-bundle-handle]')
    if (!handle || !node.contains(handle)) return
    const slotEl = node.closest<HTMLElement>('[data-bundle-item]')
    const list = slotEl?.parentElement
    if (!slotEl || !list || slotEl.dataset.bundleItem === OTHER_KEY) return
    session = {
      node,
      param,
      sx: e.clientX,
      sy: e.clientY,
      px: e.clientX,
      py: e.clientY,
      moved: false,
      list,
      scroller: node.closest<HTMLElement>('.today'),
      scroll0: 0,
      self: null as unknown as Slot,
      movable: [],
      fromIndex: 0,
      index: 0,
      ghost: null,
      offset: 0,
      headerH: 0,
      left: 0,
      top0: 0,
      gx: 0,
      gy: 0,
      raf: 0,
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('keydown', key, true)
    // Refused from the press on (handle presses only), as in assignDrag: once
    // WebKit has started a selection, setting user-select later does not undo it.
    window.addEventListener('selectstart', block, true)
    window.addEventListener('dragstart', block, true)
  }
  node.addEventListener('pointerdown', down)
  return {
    update(next: ReorderParam) {
      param = next
    },
    destroy() {
      node.removeEventListener('pointerdown', down)
      if (session?.node === node) end()
    },
  }
}

function scrollTop(s: Session): number {
  return s.scroller?.scrollTop ?? 0
}

function begin(s: Session) {
  s.moved = true
  s.scroll0 = scrollTop(s)
  const key = s.node.closest<HTMLElement>('[data-bundle-item]')?.dataset.bundleItem
  const all = [...s.list.querySelectorAll<HTMLElement>(':scope > [data-bundle-item]')].map((el) => {
    const r = el.getBoundingClientRect()
    return { el, key: el.dataset.bundleItem ?? '', top: r.top + s.scroll0, height: r.height, size: r.height }
  })
  all.forEach((slot, i) => {
    if (i < all.length - 1) slot.size = all[i + 1].top - slot.top
  })
  s.movable = all.filter((x) => x.key !== OTHER_KEY)
  const self = s.movable.find((x) => x.key === key)
  if (!self) {
    end()
    return
  }
  s.self = self
  s.fromIndex = s.index = s.movable.indexOf(self)

  const r = s.node.getBoundingClientRect()
  s.offset = r.top - (self.top - s.scroll0)
  s.headerH = r.height
  s.left = r.left
  s.top0 = r.top

  const ghost = s.node.cloneNode(true) as HTMLElement
  ghost.querySelectorAll('[id]').forEach((n) => n.removeAttribute('id'))
  ghost.removeAttribute('id')
  ghost.setAttribute('aria-hidden', 'true')
  ghost.inert = true
  Object.assign(ghost.style, {
    position: 'fixed',
    left: '0',
    top: '0',
    width: `${r.width}px`,
    margin: '0',
    zIndex: '75',
    pointerEvents: 'none',
    background: 'var(--card-surface)',
    boxShadow: 'var(--card-shadow)',
    borderRadius: '12px',
    willChange: 'transform',
  })
  document.body.appendChild(ghost)
  s.ghost = ghost

  self.el.setAttribute('data-reorder-self', '')
  s.list.setAttribute('data-reordering', '')
  document.body.setAttribute('data-bundle-drag', '')
  setUserSelect('none')
  getSelection()?.removeAllRanges()
  s.raf = requestAnimationFrame(autoscroll)
}

function move(e: PointerEvent) {
  const s = session
  if (!s) return
  s.px = e.clientX
  s.py = e.clientY
  if (!s.moved) {
    if (Math.hypot(e.clientX - s.sx, e.clientY - s.sy) < THRESHOLD) return
    begin(s)
    if (!session) return
  }
  update(s)
}

function setGhost(s: Session, x: number, y: number) {
  s.gx = x
  s.gy = y
  if (s.ghost) s.ghost.style.transform = `translate(${x}px, ${y}px)`
}

function update(s: Session) {
  setGhost(s, s.left, s.top0 + (s.py - s.sy))
  const center = s.top0 + (s.py - s.sy) + scrollTop(s) + s.headerH / 2
  // Compared with where the others sit in the original layout, using about
  // the height of a header line even for open (tall) bundles.
  const index = s.movable.filter((o) => o !== s.self && o.top + Math.min(o.height, 44) / 2 < center).length
  if (index !== s.index || !s.self.el.style.transform) {
    s.index = index
    layout(s, index)
  }
}

/** Slides every slot to where it belongs when the dragged one sits at `index`. */
function layout(s: Session, index: number) {
  const order = s.movable.filter((o) => o !== s.self)
  order.splice(index, 0, s.self)
  let y = s.movable[0].top
  for (const slot of order) {
    const dy = y - slot.top
    slot.el.style.transform = dy === 0 ? 'translateY(0)' : `translateY(${dy}px)`
    y += slot.size
  }
}

/** Where the dragged header ends up in the viewport for `index`. */
function landing(s: Session, index: number): number {
  const order = s.movable.filter((o) => o !== s.self)
  order.splice(index, 0, s.self)
  let y = s.movable[0].top
  for (const slot of order) {
    if (slot === s.self) break
    y += slot.size
  }
  return y + s.offset - scrollTop(s)
}

/** Near the top or bottom edge of the view the page scrolls along. */
function autoscroll() {
  const s = session
  if (!s) return
  if (s.scroller) {
    const r = s.scroller.getBoundingClientRect()
    let dy = 0
    if (s.py < r.top + EDGE) dy = -Math.ceil((r.top + EDGE - s.py) / 6)
    else if (s.py > r.bottom - EDGE) dy = Math.ceil((s.py - (r.bottom - EDGE)) / 6)
    if (dy) {
      s.scroller.scrollTop += dy
      update(s)
    }
  }
  s.raf = requestAnimationFrame(autoscroll)
}

function up() {
  const s = session
  if (!s) return
  if (!s.moved) {
    end()
    return
  }
  suppressClick()
  const index = s.index
  const changed = index !== s.fromIndex
  let commit: (() => void) | null = null
  if (changed) {
    const order = s.movable.filter((o) => o !== s.self)
    order.splice(index, 0, s.self)
    const next = order[index + 1]
    const prev = order[index - 1]
    const param = s.param
    if (next) commit = () => param.onreorder(next.key, 'before')
    else if (prev) commit = () => param.onreorder(prev.key, 'after')
  }
  glide(s, landing(s, index), commit)
}

function cancel() {
  const s = session
  if (!s) return
  if (!s.moved) {
    end()
    return
  }
  // Everything slides back: the gap closes where the header came from.
  layout(s, s.fromIndex)
  glide(s, landing(s, s.fromIndex), null)
}

function key(e: KeyboardEvent) {
  if (e.key !== 'Escape' || !session) return
  e.preventDefault()
  e.stopPropagation()
  cancel()
}

/** The ghost glides to its slot; then the drag is over and `done` may save. */
function glide(s: Session, top: number, done: (() => void) | null) {
  stopListening()
  cancelAnimationFrame(s.raf)
  const finish = () => {
    end()
    done?.()
  }
  if (!s.ghost || reduceMotion()) {
    finish()
    return
  }
  const anim = s.ghost.animate(
    [
      { transform: `translate(${s.gx}px, ${s.gy}px)` },
      { transform: `translate(${s.left}px, ${top}px)` },
    ],
    { duration: 200, easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)', fill: 'forwards' },
  )
  anim.onfinish = finish
  anim.oncancel = finish
}

function stopListening() {
  window.removeEventListener('pointermove', move)
  window.removeEventListener('pointerup', up)
  window.removeEventListener('pointercancel', cancel)
  window.removeEventListener('keydown', key, true)
  window.removeEventListener('selectstart', block, true)
  window.removeEventListener('dragstart', block, true)
}

function block(e: Event) {
  e.preventDefault()
}

function end() {
  const s = session
  stopListening()
  if (!s) return
  cancelAnimationFrame(s.raf)
  // Dropping the attribute removes the slide transition in the same style
  // recalculation as the transforms, so the slots do not animate back.
  s.list.removeAttribute('data-reordering')
  for (const slot of s.movable) slot.el.style.transform = ''
  s.self?.el.removeAttribute('data-reorder-self')
  s.ghost?.remove()
  document.body.removeAttribute('data-bundle-drag')
  setUserSelect('')
  session = null
}

/** A drag ends with a click on the handle's surroundings; it must not toggle the bundle. */
function suppressClick() {
  const stop = (ev: Event) => {
    ev.stopPropagation()
    ev.preventDefault()
  }
  window.addEventListener('click', stop, { capture: true, once: true })
  setTimeout(() => window.removeEventListener('click', stop, true), 60)
}

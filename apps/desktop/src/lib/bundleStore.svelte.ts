// Reactive state for bundles and placements, shared by App.svelte (feeds the
// "Heute" model) and the components that change them.

import * as api from './api'
import { OTHER_KEY, orderedBundles, type Bundle, type BundlePatch, type Placement, type Target } from './bundles'

export class BundleStore {
  bundles = $state<Bundle[]>([])
  placements = $state<Map<number, Placement>>(new Map())
  /** Last error worth showing to the user (German), cleared on the next success. */
  error = $state<string | null>(null)

  /** The first bundle list and the first placements have arrived (or failed). */
  bundlesLoaded = $state(false)
  placementsLoaded = $state(false)
  /**
   * False until the first complete result: the bundle area shows a quiet
   * placeholder then, so mail does not appear line by line. Later reloads
   * keep the previous content on screen until the new result is there.
   */
  get ready(): boolean {
    return this.bundlesLoaded && this.placementsLoaded
  }

  #ids: number[] = []
  #seq = 0
  #moving = 0

  async loadBundles(): Promise<void> {
    try {
      this.bundles = await api.bundles()
    } catch (e) {
      console.warn('Bündel konnten nicht geladen werden', e)
    } finally {
      this.bundlesLoaded = true
    }
  }

  /** Placements for exactly these mails (one batched call). */
  async loadPlacements(ids: number[]): Promise<void> {
    this.#ids = ids
    const seq = ++this.#seq
    try {
      const list = await api.placements(ids)
      if (seq !== this.#seq) return
      this.placements = new Map(list.map((p) => [p.message_id, p]))
      this.placementsLoaded = true
    } catch (e) {
      console.warn('Einordnung konnte nicht geladen werden', e)
      // A failed first call must not keep the placeholder up forever.
      if (seq === this.#seq) this.placementsLoaded = true
    }
  }

  /** After a change: bundles and the placements of the mail on screen. */
  async reload(): Promise<void> {
    await Promise.all([this.loadBundles(), this.loadPlacements(this.#ids)])
  }

  async apply(
    messageId: number,
    target: Target,
    kindKeys: string[],
    excludedIds: number[],
  ): Promise<number[]> {
    const ids = await api.applyAssignment(messageId, target, kindKeys, excludedIds)
    await this.reload()
    return ids
  }

  async undo(assignmentIds: number[]): Promise<void> {
    for (const id of assignmentIds) {
      try {
        await api.removeAssignment(id)
      } catch (e) {
        console.warn('Zuordnung konnte nicht zurückgenommen werden', e)
      }
    }
    await this.reload()
  }

  async create(name: string): Promise<Bundle | null> {
    try {
      const b = await api.createBundle(name)
      this.error = null
      await this.loadBundles()
      return b
    } catch (e) {
      this.error = String(e)
      return null
    }
  }

  async update(key: string, patch: BundlePatch): Promise<void> {
    try {
      await api.updateBundle(key, patch)
      this.error = null
    } catch (e) {
      this.error = String(e)
    }
    await this.reload()
  }

  /**
   * Moves a bundle directly before or after another one. The new order is
   * shown at once; if saving fails it is taken back and the error is shown.
   * `other` is never moved and nothing can go below it.
   */
  async reorder(key: string, anchorKey: string, side: 'before' | 'after'): Promise<void> {
    if (key === OTHER_KEY || key === anchorKey) return
    const rest = orderedBundles(this.bundles).filter((b) => b.key !== OTHER_KEY && b.key !== key)
    let position = rest.findIndex((b) => b.key === anchorKey)
    const moved = this.bundles.find((b) => b.key === key)
    if (position < 0 || !moved) return
    if (side === 'after') position += 1
    const before = this.bundles
    const order = [...rest.slice(0, position), moved, ...rest.slice(position)]
    const place = new Map(order.map((b, i) => [b.key, i]))
    // The same renumbering the core does, so nothing jumps when the saved list arrives.
    this.bundles = before.map((b) => (place.has(b.key) ? { ...b, position: place.get(b.key)! } : b))
    this.#moving += 1
    try {
      await api.updateBundle(key, { position })
      this.error = null
    } catch (e) {
      this.bundles = before
      this.error = String(e)
    } finally {
      this.#moving -= 1
      // Only the last of several quick moves reloads, so an early answer
      // cannot flip the list back under a later move.
      if (this.#moving === 0) await this.loadBundles()
    }
  }

  async remove(key: string): Promise<void> {
    try {
      await api.deleteBundle(key)
      this.error = null
    } catch (e) {
      this.error = String(e)
    }
    await this.reload()
  }
}

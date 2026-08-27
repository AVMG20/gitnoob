import { reactive } from 'vue'

/**
 * What is currently being dragged.
 *
 * The HTML drag-and-drop API can only carry strings, and reading them during
 * `dragover` is unreliable across engines, so the payload lives here and the
 * event is used only for the gesture itself.
 */
export type Payload =
  | { kind: 'branch'; name: string; remote: boolean }
  | { kind: 'commit'; oid: string; short: string; summary: string }
  | { kind: 'stash'; index: number; message: string }
  | { kind: 'tag'; name: string }
  | { kind: 'file'; path: string; staged: boolean }

const state = reactive({
  payload: null as Payload | null,
  /** Identifier of whatever the pointer is over, so it can highlight itself. */
  over: null as string | null
})

export function useDragDrop() {
  return {
    state,
    begin(event: DragEvent, payload: Payload) {
      state.payload = payload
      state.over = null
      if (event.dataTransfer) {
        event.dataTransfer.effectAllowed = 'move'
        // Something has to be set or Firefox cancels the drag.
        event.dataTransfer.setData('text/plain', describe(payload))
      }
    },
    end() {
      state.payload = null
      state.over = null
    },
    /** Marks a drop zone as hovered, if it accepts what is being dragged. */
    hover(event: DragEvent, id: string, accepts: Payload['kind'][]) {
      if (!state.payload || !accepts.includes(state.payload.kind)) return false
      event.preventDefault()
      state.over = id
      return true
    },
    /**
     * Clears the highlight when the pointer really has left the zone.
     *
     * `dragleave` also fires when the pointer crosses from a row onto the icon
     * or the label inside it, which is not leaving at all — taking it at face
     * value makes the highlight flicker off and on as the pointer moves across
     * a row. The element being entered says which it is.
     */
    leave(event: DragEvent, id: string) {
      const zone = event.currentTarget as HTMLElement | null
      const entering = event.relatedTarget as Node | null
      if (zone && entering && zone.contains(entering)) return
      if (state.over === id) state.over = null
    },
    /**
     * Returns the payload if this zone accepts it, and clears the drag.
     *
     * Generic over the kinds asked for, so what comes back is narrowed to
     * those: a zone that takes `['branch', 'commit']` gets a branch or a
     * commit, and can rule the commit out and read the branch's name. Typed as
     * the whole union it would hand back a file or a tag the caller has already
     * excluded by asking, and every field access after that needs a guard for
     * a case that cannot happen.
     */
    take<K extends Payload['kind']>(accepts: K[]): Extract<Payload, { kind: K }> | null {
      const payload = state.payload
      state.payload = null
      state.over = null
      if (!payload || !accepts.includes(payload.kind as K)) return null
      return payload as Extract<Payload, { kind: K }>
    }
  }
}

export function describe(payload: Payload) {
  switch (payload.kind) {
    case 'branch':
      return payload.name
    case 'commit':
      return payload.short
    case 'stash':
      return payload.message
    case 'tag':
      return payload.name
    case 'file':
      return payload.path
  }
}

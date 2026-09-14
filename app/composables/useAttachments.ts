import { getCurrentInstance, onUnmounted, ref } from 'vue'

import { invoke } from '~/composables/useInvoke'

/**
 * Dropping a screenshot into what you are writing.
 *
 * The gesture is the whole feature: a review with a picture in it says in one
 * frame what a paragraph says badly, and the way anyone has ever done that is
 * to drag the file onto the box. Doing it here used to mean opening the forge
 * in a browser, doing it there, and copying the markdown back.
 *
 * Both forges take the file over their API, so what is left is the window's
 * half: read the bytes, put a placeholder where the caret was so the writing
 * can carry on while it uploads, and swap the placeholder for the address when
 * it lands. A failure takes its placeholder with it rather than leaving a line
 * of text nobody typed.
 */

/** What the backend will take, so a file that cannot go says so at once. */
const KINDS: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'image/svg+xml': 'svg'
}

/** The same ten megabytes the backend stops at, said before the trip. */
const SIZE_LIMIT = 10 * 1024 * 1024

/** Whether this file is one that can be attached at all. */
export function attachable(file: File): boolean {
  const claimed = file.type.split(';')[0]?.trim().toLowerCase() ?? ''
  if (KINDS[claimed]) return true
  // Dragged out of an application that said nothing about the type.
  const suffix = file.name.split('.').pop()?.toLowerCase() ?? ''
  return Object.values(KINDS).includes(suffix === 'jpeg' ? 'jpg' : suffix)
}

/** The images in a drop or a paste, in the order they were handed over. */
export function imagesIn(source: DataTransfer | null | undefined): File[] {
  if (!source) return []
  const files = Array.from(source.files ?? [])
  // A pasted screenshot is an item rather than a file on some platforms.
  if (!files.length && source.items) {
    for (const item of Array.from(source.items)) {
      if (item.kind !== 'file') continue
      const file = item.getAsFile()
      if (file) files.push(file)
    }
  }
  return files.filter(attachable)
}

/** Whether a drag is carrying files rather than one of the app's own rows. */
export function carriesFiles(source: DataTransfer | null | undefined): boolean {
  return Array.from(source?.types ?? []).includes('Files')
}

/** The alt text a file stands in with, which is its name without the suffix. */
function altFor(file: File): string {
  const base = file.name.replace(/\.[^.]+$/, '').replace(/\./g, ' ').trim()
  return base || 'image'
}

/** Markdown does not escape inside a label, so a bracket has to be. */
function escapeAlt(alt: string): string {
  return alt.replace(/([[\]])/g, '\\$1')
}

/**
 * The line that holds the place while the file is on its way.
 *
 * Numbered, because two files dropped together are two placeholders and the
 * second must not be swapped for the first one's address.
 */
function placeholderFor(alt: string, ticket: number): string {
  return `![${escapeAlt(alt)}](uploading-${ticket})`
}

let tickets = 0

interface Field {
  /** The box being written in, for the caret. */
  field: () => HTMLTextAreaElement | null
  /** What it currently holds. */
  text: () => string
  /** What it should hold instead. */
  write: (value: string) => void
}

/**
 * Wires one text box up to accept dropped and pasted images.
 *
 * Given to a component rather than done by it, because there are four boxes a
 * body is written in — a new request, an edited description, a comment and a
 * reply — and the gesture has to mean the same thing in all of them.
 */
export function useAttachments(box: Field) {
  /** Whether something droppable is over the box, for the outline. */
  const over = ref(false)
  /** How many files are still on their way. */
  const sending = ref(0)
  /** Whatever the last one failed with, for the box to say. */
  const failure = ref<string | null>(null)

  /**
   * Whether the box that asked for this is still on screen.
   *
   * An upload outlives its box — a reply folded away, another thread opened —
   * and where the caller holds the text, writing to a box that has gone is a
   * write Vue drops on the floor. The placeholder would then stay in the
   * caller's draft for good, and in the case of a line remark it is written to
   * disk that way. So a settle that arrives too late does nothing but say so.
   */
  let live = true
  /** The placeholders still standing in for something on its way. */
  const outstanding: { run: string; placeholder: string }[] = []
  if (getCurrentInstance()) {
    onUnmounted(() => {
      // Taken back while the box is still there to take them back from: an
      // unmount hook still reaches the caller, a moment later does not.
      for (const one of outstanding.splice(0)) settle(one.run, one.placeholder, null)
      live = false
    })
  }

  /**
   * Writes `text` where the caret is, and leaves the caret after it.
   *
   * Answers with the whole run it added, breaks included, because a failed
   * upload has to take back exactly what it put in — a placeholder removed on
   * its own leaves the blank line it was given behind.
   */
  function insert(text: string): string {
    const field = box.field()
    const held = box.text()
    const at = field ? field.selectionStart : held.length
    const to = field ? field.selectionEnd : held.length
    // A picture wants a line of its own, unless it already has one.
    const before = held.slice(0, at)
    const after = held.slice(to)
    const lead = before && !before.endsWith('\n') ? '\n' : ''
    const trail = after.startsWith('\n') || !after ? '' : '\n'
    const run = `${lead}${text}${trail}`
    box.write(`${before}${run}${after}`)
    const caret = before.length + lead.length + text.length
    // The value is the caller's, so the box only has it after Vue writes it.
    void nextTickCaret(field, caret)
    return run
  }

  async function nextTickCaret(field: HTMLTextAreaElement | null, caret: number, take = true) {
    if (!field) return
    await Promise.resolve()
    field.selectionStart = caret
    field.selectionEnd = caret
    if (take) field.focus()
  }

  /**
   * Swaps a placeholder for what it stood in for, or takes the whole run back.
   *
   * By searching rather than by remembering where it was: whoever dropped the
   * file went on typing while it uploaded, and everything after the caret has
   * moved by however much they wrote.
   *
   * Which is also why the caret is put back afterwards. Writing a textarea's
   * value sends the caret to the end of it, and the whole point of a
   * placeholder is that the writing carries on while the file uploads — so the
   * swap landing would otherwise drop the next keystroke at the bottom of the
   * comment, with nothing on screen to explain it.
   */
  function settle(run: string, placeholder: string, markdown: string | null) {
    const held = box.text()
    const wanted = markdown === null && held.includes(run) ? run : placeholder
    const at = held.indexOf(wanted)
    if (at === -1) return
    const instead = markdown === null ? '' : markdown

    const field = box.field()
    const caret = field ? field.selectionStart : null
    // A function replacer, because a string one still reads `$&` and `$'` out
    // of the replacement — and the replacement carries a dropped file's name.
    box.write(held.replace(wanted, () => instead))

    if (caret === null) return
    // Only what sat after the swap moves, and by however much it changed.
    const moved = caret <= at ? caret : caret + (instead.length - wanted.length)
    const length = held.length - wanted.length + instead.length
    void nextTickCaret(field, Math.max(0, Math.min(moved, length)), false)
  }

  /** The file as the text the call carries it in. */
  async function encode(file: File): Promise<string> {
    const bytes = new Uint8Array(await file.arrayBuffer())
    // Chunked, because spreading ten million arguments into one call is how
    // this throws a range error on a large screenshot — and the window is let
    // go of every couple of megabytes, so the spinner beside it still turns.
    const parts: string[] = []
    const step = 0x8000
    for (let at = 0; at < bytes.length; at += step) {
      parts.push(String.fromCharCode(...bytes.subarray(at, at + step)))
      if (parts.length % 64 === 0) await new Promise((done) => setTimeout(done, 0))
    }
    return btoa(parts.join(''))
  }

  async function send(file: File) {
    const alt = altFor(file)
    const ticket = (tickets += 1)
    const placeholder = placeholderFor(alt, ticket)
    const run = insert(placeholder)
    const pending = { run, placeholder }
    outstanding.push(pending)
    sending.value += 1
    try {
      if (file.size > SIZE_LIMIT) {
        throw new Error(`${file.name} is larger than 10 MB`)
      }
      const url = await invoke<string>('forge_upload_attachment', {
        name: file.name,
        mime: file.type,
        data: await encode(file)
      })
      if (live) settle(run, placeholder, `![${escapeAlt(alt)}](${url})`)
      failure.value = null
    } catch (error) {
      if (live) settle(run, placeholder, null)
      failure.value = String(error).replace(/^Error:\s*/, '')
    } finally {
      const at = outstanding.indexOf(pending)
      if (at !== -1) outstanding.splice(at, 1)
      sending.value -= 1
    }
  }

  /** Takes what was handed over, and answers whether any of it was taken. */
  async function take(source: DataTransfer | null | undefined): Promise<boolean> {
    const files = imagesIn(source)
    if (!files.length) return false
    // Whatever went wrong last time is about last time.
    failure.value = null
    // One at a time, so two placeholders cannot race for the same caret.
    for (const file of files) await send(file)
    return true
  }

  return {
    over,
    sending,
    failure,
    take,

    onDragOver(event: DragEvent) {
      if (!carriesFiles(event.dataTransfer)) return
      event.preventDefault()
      if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
      over.value = true
    },

    onDragLeave(event: DragEvent) {
      const zone = event.currentTarget as HTMLElement | null
      const entering = event.relatedTarget as Node | null
      // Crossing from the box onto the buttons inside it is not leaving.
      if (zone && entering && zone.contains(entering)) return
      over.value = false
    },

    async onDrop(event: DragEvent) {
      if (!carriesFiles(event.dataTransfer)) return
      event.preventDefault()
      over.value = false
      const took = await take(event.dataTransfer)
      // The box lit up, so something has to be said about why nothing landed.
      if (!took) failure.value = 'Only PNG, JPEG, GIF, WebP and SVG images can be attached'
    },

    async onPaste(event: ClipboardEvent) {
      const files = imagesIn(event.clipboardData)
      if (!files.length) return
      // Only when it really is a picture: a paste that is text stays a paste.
      event.preventDefault()
      await take(event.clipboardData)
    }
  }
}

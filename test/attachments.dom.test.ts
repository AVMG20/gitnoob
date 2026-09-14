// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import CommentBox from '~/components/CommentBox.vue'
import { attachable, imagesIn, carriesFiles } from '~/composables/useAttachments'

/** What the backend answers with, or throws, for the next upload. */
let answer: { url?: string; fail?: string } = { url: 'https://example.com/a/1' }

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd !== 'forge_upload_attachment') return null
    if (answer.fail) throw new Error(answer.fail)
    return answer.url
  })
}))

/**
 * A dropped file, as the window hands one over.
 *
 * happy-dom has no `DataTransfer` worth using, and the composable only ever
 * reads `types` and `files`, so this is the whole of what it sees.
 */
function dropped(files: File[]): DataTransfer {
  return {
    types: files.length ? ['Files'] : ['text/plain'],
    files,
    items: files.map((file) => ({ kind: 'file', getAsFile: () => file }))
  } as unknown as DataTransfer
}

function png(name = 'shot.png', size = 12): File {
  const file = new File([new Uint8Array(size)], name, { type: 'image/png' })
  // happy-dom's File does not implement it, and the composable reads the bytes.
  Object.defineProperty(file, 'arrayBuffer', {
    value: async () => new Uint8Array(size).buffer
  })
  return file
}

beforeEach(() => {
  answer = { url: 'https://example.com/a/1' }
  vi.mocked(invoke).mockClear()
})

describe('what can be attached', () => {
  it('takes the image kinds both forges take, and nothing else', () => {
    expect(attachable(new File([], 'a.png', { type: 'image/png' }))).toBe(true)
    expect(attachable(new File([], 'a.jpg', { type: 'image/jpeg' }))).toBe(true)
    expect(attachable(new File([], 'a.webp', { type: 'image/webp' }))).toBe(true)
    expect(attachable(new File([], 'a.svg', { type: 'image/svg+xml' }))).toBe(true)
    // Dragged from somewhere that said nothing about the type.
    expect(attachable(new File([], 'a.JPEG', { type: '' }))).toBe(true)
    expect(attachable(new File([], 'notes.pdf', { type: 'application/pdf' }))).toBe(false)
    expect(attachable(new File([], 'clip.mp4', { type: 'video/mp4' }))).toBe(false)
    expect(attachable(new File([], 'run.sh', { type: '' }))).toBe(false)
  })

  it('picks the images out of a drop and leaves the rest', () => {
    const shot = png()
    const notes = new File([], 'notes.pdf', { type: 'application/pdf' })
    expect(imagesIn(dropped([shot, notes]))).toEqual([shot])
    expect(imagesIn(null)).toEqual([])
  })

  it('knows a drag carrying files from one carrying a branch', () => {
    expect(carriesFiles(dropped([png()]))).toBe(true)
    expect(carriesFiles(dropped([]))).toBe(false)
    expect(carriesFiles(null)).toBe(false)
  })
})

describe('dropping an image on a comment box', () => {
  it('sends the file and writes the markdown where the caret was', async () => {
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.setValue('Looks off here:')

    await field.trigger('drop', { dataTransfer: dropped([png('a shot.png')]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith(
      'forge_upload_attachment',
      expect.objectContaining({ name: 'a shot.png', mime: 'image/png' })
    )
    const written = (field.element as HTMLTextAreaElement).value
    expect(written).toBe('Looks off here:\n![a shot](https://example.com/a/1)')
    // Nothing is left holding the place once it has landed.
    expect(written).not.toContain('uploading-')
  })

  it('takes its placeholder back when the upload fails, and says why', async () => {
    answer = { fail: 'Attaching files needs push access to this repository' }
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.setValue('before')

    await field.trigger('drop', { dataTransfer: dropped([png()]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()

    expect((field.element as HTMLTextAreaElement).value).toBe('before')
    expect(wrapper.find('[data-testid="attach-failure"]').text()).toContain('push access')
  })

  it('leaves a drag that is not a file alone, so a branch still drops through', async () => {
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.trigger('drop', { dataTransfer: dropped([]) })
    await wrapper.vm.$nextTick()
    expect(vi.mocked(invoke)).not.toHaveBeenCalled()
    expect(wrapper.find('[data-testid="attach-failure"]').exists()).toBe(false)
  })

  it('says why nothing landed when the file dropped was not an image', async () => {
    const wrapper = mount(CommentBox)
    const notes = new File([], 'notes.pdf', { type: 'application/pdf' })
    await wrapper.find('textarea').trigger('drop', { dataTransfer: dropped([notes]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    expect(vi.mocked(invoke)).not.toHaveBeenCalled()
    expect(wrapper.find('[data-testid="attach-failure"]').text()).toContain('PNG')
  })

  it('writes two dropped images one after the other, each with its own line', async () => {
    vi.mocked(invoke)
      .mockImplementationOnce(async () => 'https://example.com/a/1')
      .mockImplementationOnce(async () => 'https://example.com/a/2')
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.setValue('two of them:')

    await field.trigger('drop', { dataTransfer: dropped([png('one.png'), png('two.png')]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()

    expect((field.element as HTMLTextAreaElement).value).toBe(
      'two of them:\n![one](https://example.com/a/1)\n![two](https://example.com/a/2)'
    )
  })

  it('refuses to send while a picture in the body is still on its way', async () => {
    let land: ((url: string) => void) | null = null
    vi.mocked(invoke).mockImplementationOnce(
      () => new Promise<string>((done) => (land = done)) as Promise<unknown>
    )
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.setValue('text')

    await field.trigger('drop', { dataTransfer: dropped([png()]) })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-testid="comment-send"]').attributes('disabled')).toBeDefined()
    expect(wrapper.find('[data-testid="attach-busy"]').exists()).toBe(true)

    land!('https://example.com/a/2')
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-testid="comment-send"]').attributes('disabled')).toBeUndefined()
  })

  it('leaves the caret where it was typing when the upload lands', async () => {
    let land: ((url: string) => void) | null = null
    vi.mocked(invoke).mockImplementationOnce(
      () => new Promise<string>((done) => (land = done)) as Promise<unknown>
    )
    const wrapper = mount(CommentBox, { attachTo: document.body })
    const field = wrapper.find('textarea')
    const element = field.element as HTMLTextAreaElement
    await field.setValue('one')

    await field.trigger('drop', { dataTransfer: dropped([png()]) })
    await wrapper.vm.$nextTick()
    // The writing carries on while it uploads, which is the whole point of
    // holding the place.
    element.selectionStart = 2
    element.selectionEnd = 2

    land!('https://example.com/a/9')
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    await new Promise((done) => setTimeout(done, 0))

    expect(element.value).toContain('https://example.com/a/9')
    // Not sent to the end of the comment by the swap.
    expect(element.selectionStart).toBe(2)
    wrapper.unmount()
  })

  it('says nothing about a failure that was two drops ago', async () => {
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    const notes = new File([], 'notes.pdf', { type: 'application/pdf' })
    await field.trigger('drop', { dataTransfer: dropped([notes]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-testid="attach-failure"]').exists()).toBe(true)

    await field.trigger('drop', { dataTransfer: dropped([png()]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-testid="attach-failure"]').exists()).toBe(false)
  })

  it('does not let a name with a dollar in it rewrite the body', async () => {
    vi.mocked(invoke).mockImplementationOnce(async () => 'https://example.com/a/3')
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.trigger('drop', { dataTransfer: dropped([png("before$&after$'.png")]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()

    const written = (field.element as HTMLTextAreaElement).value
    expect(written).toBe("![before$&after$'](https://example.com/a/3)")
    expect(written).not.toContain('uploading-')
  })

  it('marks itself while an image is over it', async () => {
    const wrapper = mount(CommentBox)
    const field = wrapper.find('textarea')
    await field.trigger('dragover', { dataTransfer: dropped([png()]) })
    expect(wrapper.find('.box').classes()).toContain('over')
    await field.trigger('dragleave', { dataTransfer: dropped([png()]) })
    expect(wrapper.find('.box').classes()).not.toContain('over')
  })
})

/**
 * Every box in the app hands the text to the caller — a reply's `answer`, a
 * line remark's saved draft — and a caller's text outlives the box. Vue drops
 * an emit from a component that has gone, so a placeholder left behind by an
 * upload that landed too late would stay in the draft for good, and in the
 * case of a line remark it is written to disk that way.
 */
describe('a comment box whose text belongs to whoever mounted it', () => {
  /** A parent holding the text, the way every real call site does. */
  const Holder = {
    components: { CommentBox },
    data: () => ({ body: '', shown: true }),
    template: '<CommentBox v-if="shown" v-model="body" />'
  }

  it('sends and swaps through the caller rather than through the box', async () => {
    vi.mocked(invoke).mockImplementationOnce(async () => 'https://example.com/a/4')
    const wrapper = mount(Holder)
    await wrapper.find('textarea').trigger('drop', { dataTransfer: dropped([png('held.png')]) })
    await new Promise((done) => setTimeout(done, 0))
    await wrapper.vm.$nextTick()
    expect((wrapper.vm as unknown as { body: string }).body).toBe(
      '![held](https://example.com/a/4)'
    )
  })

  it('takes its placeholder back rather than stranding it in a draft', async () => {
    let land: ((url: string) => void) | null = null
    vi.mocked(invoke).mockImplementationOnce(
      () => new Promise<string>((done) => (land = done)) as Promise<unknown>
    )
    const wrapper = mount(Holder)
    const held = wrapper.vm as unknown as { body: string; shown: boolean }
    held.body = 'start'
    await wrapper.vm.$nextTick()

    await wrapper.find('textarea').trigger('drop', { dataTransfer: dropped([png('late.png')]) })
    await wrapper.vm.$nextTick()
    expect(held.body).toContain('uploading-')

    // The reply is folded away while the file is still going up.
    held.shown = false
    await wrapper.vm.$nextTick()
    expect(held.body).toBe('start')

    // And what lands afterwards writes nothing into the draft it left.
    land!('https://example.com/a/5')
    await new Promise((done) => setTimeout(done, 0))
    expect(held.body).toBe('start')
  })
})

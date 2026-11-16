// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import { blameGutter, setBlame } from '~/composables/useCodeMirror'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import FileEditor from '~/components/FileEditor.vue'
import BlamePopover from '~/components/BlamePopover.vue'
import { blameDate, blamePopup, closeBlamePopup } from '~/composables/blame'
import { resetTabs } from '~/composables/useTabs'
import { diffMode } from '~/composables/useDiffMode'
import { useContextMenu } from '~/composables/useContextMenu'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const menu = useContextMenu()

const DAY = 86400
const now = Math.floor(Date.now() / 1000)
const OLD = 'a91c4e2000000000000000000000000000000000'
const NEW = 'b77d0f1000000000000000000000000000000000'

const RUNS = [
  {
    oid: OLD,
    short: 'a91c4e2',
    summary: 'feat: the first pass',
    author: 'Ramon Robben',
    email: 'ramon@example.com',
    time: now - 400 * DAY,
    start: 1,
    lines: 2,
    uncommitted: false
  },
  {
    oid: NEW,
    short: 'b77d0f1',
    summary: 'fix: rounding',
    author: 'Ann Other',
    email: 'ann@example.com',
    time: now - 3 * DAY,
    start: 3,
    lines: 2,
    uncommitted: false
  }
]

let calls: { cmd: string; args: Record<string, unknown> }[] = []

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'read_file') {
      return { text: 'one\ntwo\nthree\nfour\n', hash: 'h1', eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'file_text') return 'one\ntwo\nthree\nfour\n'
    if (cmd === 'working_file_diff') return { path: 'app/app.vue', binary: false, truncated: 0, hunks: [] }
    if (cmd === 'blame_file') return RUNS
    if (cmd === 'commit_depth') return 1
    if (cmd === 'commit_detail') {
      const run = RUNS.find((one) => one.oid === args?.oid) ?? RUNS[0]!
      return { ...run, body: 'Rounded half to even.\nThe old way lost a cent.', committer: run.author, commit_time: run.time, parents: [], files: [] }
    }
    if (cmd === 'write_file') return 'h2'
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [{ path: 'app/app.vue' }], conflicted: [] } as never
  git.store.viewer = { path: 'app/app.vue', side: 'unstaged' } as never
  diffMode.mode = 'file'
  diffMode.blame = false
})

let open: ReturnType<typeof mount> | null = null
let popover: ReturnType<typeof mount> | null = null

/** Where app.vue puts the popover: on top of everything, while it is open. */
const Host = defineComponent({ setup: () => () => (blamePopup.open ? h(BlamePopover) : null) })

afterEach(() => {
  menu.close()
  closeBlamePopup()
  popover?.unmount()
  popover = null
  open?.unmount()
  open = null
  diffMode.mode = 'diff'
  diffMode.blame = false
  git.store.viewer = null
})

const show = async () => {
  open = mount(DiffViewer, {
    attachTo: document.body,
    global: { components: { FileView, FileEditor }, stubs: { Spinner: true, ChangeRuler: true } }
  })
  await flushPromises()
  return open
}

const rightClick = (element: Element) =>
  element.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))

/** Each annotated line's text, in order, empty for the lines under a run's first. */
const column = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.cm-blame-entry').map((entry) => entry.text())

/** Right-click the numbers and choose "Annotate with Git Blame". */
async function annotate(wrapper: ReturnType<typeof mount>) {
  rightClick(wrapper.find('.cm-lineNumbers .cm-gutterElement:not([style*="hidden"])').element)
  expect(menu.state.open).toBe(true)
  const item = menu.state.items.find((one) => one.label === 'Annotate with Git Blame')
  expect(item).toBeDefined()
  item!.action?.()
  menu.close()
  await flushPromises()
}

describe('annotating the editor with git blame', () => {
  it('turns on from the line numbers and shows each run once, date and author', async () => {
    const wrapper = await show()
    expect(wrapper.find('.cm-blame-on').exists()).toBe(false)
    await annotate(wrapper)

    expect(diffMode.blame).toBe(true)
    expect(calls.filter((call) => call.cmd === 'blame_file')).toHaveLength(1)
    expect(wrapper.find('.cm-blame-on').exists()).toBe(true)
    // Left of the numbers, where an IDE puts it.
    const gutters = wrapper.findAll('.cm-gutter').map((one) => one.classes())
    expect(gutters[0]).toContain('cm-blameGutter')
    expect(gutters[1]).toContain('cm-lineNumbers')
    // The first line of each run says who and when; the rest of it says
    // nothing, and the empty line after the last newline has no entry.
    expect(column(wrapper)).toEqual([
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      `${blameDate(RUNS[1]!.time)}Ann Other`,
      ''
    ])
    // Older work is faded further than newer.
    const ages = wrapper
      .findAll('.cm-blame-entry')
      .map((entry) => Number((entry.element as HTMLElement).style.getPropertyValue('--blame-age')))
    expect(ages[0]).toBeLessThan(ages[2]!)

    // And off again from the same menu, which the bar's button agrees with.
    rightClick(wrapper.find('.cm-lineNumbers .cm-gutterElement:not([style*="hidden"])').element)
    const close = menu.state.items.find((one) => one.label === 'Close Annotations')
    expect(close).toBeDefined()
    close!.action?.()
    await flushPromises()
    expect(diffMode.blame).toBe(false)
    expect(wrapper.find('.cm-blame-on').exists()).toBe(false)
  })

  it('says the subject, hash, author and address on hover', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    const title = wrapper.findAll('.cm-blame-entry')[2]!.attributes('title')!
    expect(title).toContain('Ann Other <ann@example.com>')
    expect(title).toContain('b77d0f1 fix: rounding')
  })

  /** Click an entry; the popover opens beside it and the file stays. */
  async function clickEntry(wrapper: ReturnType<typeof mount>, selector = '.cm-blame-entry.head') {
    popover ??= mount(Host, { attachTo: document.body })
    // happy-dom lays nothing out, so the click lands on the first line.
    wrapper
      .find(selector)
      .element.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, clientY: 0 }))
    await flushPromises()
  }

  const pop = () => document.querySelector<HTMLElement>('[data-testid="blame-popover"]')

  it('opens a popover on the entry when it is clicked, and stays in the file', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    await clickEntry(wrapper)

    expect(blamePopup.open).toBe(true)
    const panel = pop()!
    expect(panel).not.toBeNull()
    expect(panel.textContent).toContain('Ramon Robben')
    expect(panel.textContent).toContain('ramon@example.com')
    expect(panel.textContent).toContain('a91c4e2')
    expect(panel.textContent).toMatch(/1 year ago/)
    // The whole message, not only the subject.
    expect(panel.querySelector('[data-testid="blame-message"]')!.textContent).toBe(
      'feat: the first pass\n\nRounded half to even.\nThe old way lost a cent.'
    )
    // Read through libgit2: no `git log` in the activity for a click.
    expect(calls.some((call) => call.cmd === 'commit_message_text')).toBe(false)
    // Nowhere else: the graph was not asked for, and the file is still open.
    expect(git.store.revealing?.oid).not.toBe(OLD)
    expect(git.store.viewer?.path).toBe('app/app.vue')
    expect(wrapper.find('.cm-editor').exists()).toBe(true)
    // And it is focused, so Esc reaches it.
    expect(document.activeElement).toBe(panel)
  })

  it('closes on Esc and hands focus back to the editor', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    await clickEntry(wrapper)
    pop()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    await flushPromises()
    expect(blamePopup.open).toBe(false)
    expect(pop()).toBeNull()
    expect(document.activeElement?.closest('.cm-editor')).not.toBeNull()
    // Esc was the popover's: the file is still on screen, not the graph.
    expect(git.store.viewer?.path).toBe('app/app.vue')
  })

  it('closes on a click outside and on a scroll of the editor', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    await clickEntry(wrapper)
    // A click inside does not close it.
    pop()!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    expect(blamePopup.open).toBe(true)
    wrapper.find('.cm-content').element.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    await flushPromises()
    expect(blamePopup.open).toBe(false)

    await clickEntry(wrapper)
    expect(blamePopup.open).toBe(true)
    wrapper.find('.cm-scroller').element.dispatchEvent(new Event('scroll'))
    await flushPromises()
    expect(blamePopup.open).toBe(false)
  })

  it('closes when another file comes on screen', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    await clickEntry(wrapper)
    git.store.viewer = { path: 'app/other.vue', side: 'unstaged' } as never
    await flushPromises()
    expect(blamePopup.open).toBe(false)
  })

  it('shows what the commit changed in this file, in the graph, or copies its hash', async () => {
    const written = vi.fn(async () => undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: written }, configurable: true })
    const wrapper = await show()
    await annotate(wrapper)

    await clickEntry(wrapper)
    ;(pop()!.querySelector('[data-testid="blame-hash"]') as HTMLElement).click()
    await flushPromises()
    expect(written).toHaveBeenLastCalledWith(OLD)
    expect(blamePopup.open).toBe(false)

    await clickEntry(wrapper)
    ;(pop()!.querySelector('[data-testid="blame-copy"]') as HTMLElement).click()
    await flushPromises()
    expect(written).toHaveBeenCalledTimes(2)

    await clickEntry(wrapper)
    ;(pop()!.querySelector('[data-testid="blame-change"]') as HTMLElement).click()
    await flushPromises()
    expect(blamePopup.open).toBe(false)
    // The commit's copy of this file, in Diff mode against its parent.
    expect(git.store.viewer).toEqual(expect.objectContaining({ path: 'app/app.vue', commit: OLD }))
    resetTabs()
  })

  it('shows the commit in the graph from the popover, as a click used to', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    await clickEntry(wrapper)
    ;(pop()!.querySelector('[data-testid="blame-graph"]') as HTMLElement).click()
    await flushPromises()
    expect(blamePopup.open).toBe(false)
    expect(git.store.revealing?.oid).toBe(OLD)
  })

  it('does nothing on a click of a line that is not committed yet', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    const view = EditorView.findFromDOM(wrapper.find('.cm-editor').element as HTMLElement)!
    view.dispatch({ changes: { from: 0, insert: 'new\n' }, userEvent: 'input.type' })
    await flushPromises()
    await clickEntry(wrapper, '.cm-blame-entry.fresh')
    expect(blamePopup.open).toBe(false)
  })

  it('offers the commit hash on a right-click of an entry', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    rightClick(wrapper.find('.cm-blame-entry.head').element)
    const labels = menu.state.items.filter((one) => !one.separator).map((one) => one.label)
    expect(labels).toEqual(['Copy commit hash', 'Show in graph', 'Close Annotations'])
  })

  it('moves the annotations with the text, and calls new lines not committed', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    const view = EditorView.findFromDOM(wrapper.find('.cm-editor').element as HTMLElement)!

    // A line typed above everything: the rest move down with their authors.
    view.dispatch({ changes: { from: 0, insert: 'new\n' }, userEvent: 'input.type' })
    await flushPromises()
    expect(column(wrapper)).toEqual([
      'Not committed yet',
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      `${blameDate(RUNS[1]!.time)}Ann Other`,
      ''
    ])

    // Enter at the end of "two" keeps "two" Ramon's and the new line nobody's.
    const two = view.state.doc.line(3)
    view.dispatch({ changes: { from: two.to, insert: '\n' }, userEvent: 'input.type' })
    await flushPromises()
    expect(column(wrapper)).toEqual([
      'Not committed yet',
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      'Not committed yet',
      `${blameDate(RUNS[1]!.time)}Ann Other`,
      ''
    ])

    // Typing into a line makes it yours: its author is not who wrote it now.
    view.dispatch({ changes: { from: view.state.doc.line(5).from, insert: 'x' }, userEvent: 'input.type' })
    await flushPromises()
    expect(column(wrapper)).toEqual([
      'Not committed yet',
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      'Not committed yet',
      '',
      `${blameDate(RUNS[1]!.time)}Ann Other`
    ])
  })

  it('reads an answer that comes in after unsaved typing against the text it was read from', async () => {
    const wrapper = await show()
    const view = EditorView.findFromDOM(wrapper.find('.cm-editor').element as HTMLElement)!
    // Typed before the column was opened, and not saved yet: the runs are
    // for the file on disk, which does not have this line.
    view.dispatch({ changes: { from: 0, insert: 'new\n' }, userEvent: 'input.type' })
    await annotate(wrapper)
    expect(column(wrapper)).toEqual([
      'Not committed yet',
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      `${blameDate(RUNS[1]!.time)}Ann Other`,
      ''
    ])
  })

  it('calls lines an older answer does not reach not committed, never blank', async () => {
    const wrapper = await show()
    await annotate(wrapper)
    const view = EditorView.findFromDOM(wrapper.find('.cm-editor').element as HTMLElement)!
    // Two lines added at the end and saved, and the answer still the one
    // read before the save (reads overlap, and finish out of order).
    view.dispatch({ changes: { from: view.state.doc.length, insert: 'five\nsix\n' }, userEvent: 'input.type' })
    await flushPromises()
    expect(column(wrapper)).toEqual([
      `${blameDate(RUNS[0]!.time)}Ramon Robben`,
      '',
      `${blameDate(RUNS[1]!.time)}Ann Other`,
      '',
      'Not committed yet',
      ''
    ])

    // The same answer said again against the file as saved since (six lines,
    // the runs cover four): the column that followed the typing stays.
    const saved = view.state.doc.toString()
    view.dispatch({ effects: setBlame.of({ runs: RUNS, text: saved }) })
    await flushPromises()
    expect(wrapper.findAll('.cm-blame-entry')).toHaveLength(6)
    expect(column(wrapper)[0]).toContain('Ramon Robben')
    expect(column(wrapper)[4]).toBe('Not committed yet')
  })

  it('marks lines with no answer at all not committed, not blank', () => {
    const view = new EditorView({
      doc: 'one\ntwo\nthree\nfour\nfive\nsix\n',
      extensions: [blameGutter(() => undefined)],
      parent: document.body
    })
    // A first answer that is short of the text: nothing to keep, so it is laid
    // over the lines, and the two it does not reach are yours.
    view.dispatch({ effects: setBlame.of({ runs: RUNS, text: view.state.doc.toString() }) })
    const entries = [...view.dom.querySelectorAll('.cm-blame-entry')].map((one) => one.textContent)
    expect(entries).toHaveLength(6)
    expect(entries[4]).toBe('Not committed yet')
    expect(view.dom.querySelectorAll('.cm-blame-entry.fresh')).toHaveLength(2)
    view.destroy()
  })

  it('takes only the latest of two overlapping reads', async () => {
    let release: (() => void) | null = null
    const LATE = [{ ...RUNS[0]!, start: 1, lines: 4, author: 'Stale Answer' }]
    let reads = 0
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'read_file') {
        return { text: 'one\ntwo\nthree\nfour\n', hash: 'h1', eol: 'lf', bom: false, readonly: null, missing: false }
      }
      if (cmd === 'file_text') return 'one\ntwo\nthree\nfour\n'
      if (cmd === 'working_file_diff') return { path: 'app/app.vue', binary: false, truncated: 0, hunks: [] }
      if (cmd === 'blame_file') {
        reads += 1
        // The first read is slow and comes back last.
        if (reads === 1) return new Promise((done) => (release = () => done(LATE)))
        return RUNS
      }
      return null
    })
    const wrapper = await show()
    await annotate(wrapper)
    git.store.status = { staged: [], unstaged: [{ path: 'app/app.vue' }], conflicted: [] } as never
    await flushPromises()
    expect(column(wrapper)[0]).toContain('Ramon Robben')
    release!()
    await flushPromises()
    expect(column(wrapper)[0]).toContain('Ramon Robben')
    expect(wrapper.text()).not.toContain('Stale Answer')
  })

  it('closes the column to no width at all, with nothing in it to hold it open', async () => {
    const wrapper = await show()
    // CodeMirror's `.cm-gutter { display: flex !important }` beats a
    // `display: none`, which left a wide empty column: closed, it has no
    // width, no padding, no border and no entries or spacer to size it.
    const gutter = wrapper.find('.cm-blameGutter').element as HTMLElement
    // The only element in it is the active line's empty highlight, which every
    // gutter gets; no entry and no spacer.
    expect(gutter.querySelector('.cm-blame-entry')).toBeNull()
    expect([...gutter.children].every((one) => one.textContent === '' && !one.children.length)).toBe(true)
    const rules = themeRules()
    const closed = rules.filter((rule) => /\s\.cm-blameGutter$/.test(rule.selectorText) && !rule.selectorText.includes('cm-blame-on'))
    expect(closed).toHaveLength(1)
    expect(closed[0]!.style.width).toMatch(/^0(px)?$/)
    expect(closed[0]!.style.overflow).toBe('hidden')
    // Nothing else the theme says about the closed column gives it a size.
    for (const rule of closed) {
      for (const property of ['min-width', 'padding', 'padding-left', 'padding-right', 'border', 'border-right', 'border-left']) {
        expect(rule.style.getPropertyValue(property)).toBe('')
      }
    }
    // Open, the width and the rule beside it come with `.cm-blame-on`.
    // (happy-dom drops the `calc(var(...))` width when it parses the rule, so
    // only that the rule is there is checked.)
    expect(rules.some((rule) => /\.cm-blame-on \.cm-blameGutter$/.test(rule.selectorText))).toBe(true)
    await annotate(wrapper)
    expect(wrapper.find('.cm-editor').classes()).toContain('cm-blame-on')
    expect(gutter.querySelectorAll('.cm-gutterElement').length).toBeGreaterThan(0)
  })
})

/** The editor theme's style rules, wherever style-mod mounted them. */
function themeRules(): CSSStyleRule[] {
  const sheets = [
    ...((document as unknown as { adoptedStyleSheets?: CSSStyleSheet[] }).adoptedStyleSheets ?? []),
    ...[...document.querySelectorAll('style')].map((one) => (one as HTMLStyleElement).sheet).filter(Boolean)
  ] as CSSStyleSheet[]
  return sheets.flatMap((sheet) => [...sheet.cssRules]).filter((rule): rule is CSSStyleRule => 'selectorText' in rule)
}

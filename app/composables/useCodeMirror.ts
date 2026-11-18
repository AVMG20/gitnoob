import {
  Compartment,
  Facet,
  Prec,
  RangeSet,
  RangeSetBuilder,
  RangeValue,
  StateEffect,
  StateField,
  type EditorState,
  type Extension,
  type Range,
  type Text,
  type Transaction
} from '@codemirror/state'
import {
  Decoration,
  EditorView,
  GutterMarker,
  ViewPlugin,
  WidgetType,
  drawSelection,
  gutter,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  type DecorationSet,
  type KeyBinding,
  type ViewUpdate
} from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import {
  HighlightStyle,
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  type Language
} from '@codemirror/language'
import { tags as t, type Tag } from '@lezer/highlight'
import { markedLines } from './useCode'
import { occurrences, type FindMarks } from './useFind'
import { themeRules } from './useHighlight'
import { type BlameRun, type DiffHunk } from './useGit'
import { blameDate, blameTooltip } from './blame'
import { tint } from './useAvatars'

/**
 * The pieces every CodeMirror view in the app is built from: the File mode
 * editor and the search window's preview.
 *
 * Nothing here is styled with fixed colours. The chrome takes the app's CSS
 * variables, so it follows every theme, and the code takes the colours of the
 * Shiki theme the diff view uses, so a file reads the same in both modes.
 */

// --- languages, loaded when a file needs one -----------------------------------

type Loader = () => Promise<Extension>

const LANGUAGES: Record<string, Loader> = {
  php: async () => (await import('@codemirror/lang-php')).php(),
  // A Magento template: HTML with PHP in it, lang-php's mixed mode.
  phtml: async () => (await import('@codemirror/lang-php')).php({ plain: false }),
  xml: async () => (await import('@codemirror/lang-xml')).xml(),
  xsd: async () => (await import('@codemirror/lang-xml')).xml(),
  xsl: async () => (await import('@codemirror/lang-xml')).xml(),
  js: async () => (await import('@codemirror/lang-javascript')).javascript(),
  mjs: async () => (await import('@codemirror/lang-javascript')).javascript(),
  cjs: async () => (await import('@codemirror/lang-javascript')).javascript(),
  jsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true }),
  ts: async () => (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  mts: async () => (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  cts: async () => (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  tsx: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true, jsx: true }),
  vue: async () => (await import('@codemirror/lang-vue')).vue(),
  rs: async () => (await import('@codemirror/lang-rust')).rust(),
  py: async () => (await import('@codemirror/lang-python')).python(),
  go: async () => (await import('@codemirror/lang-go')).go(),
  json: async () => (await import('@codemirror/lang-json')).json(),
  css: async () => (await import('@codemirror/lang-css')).css(),
  scss: async () => (await import('@codemirror/lang-css')).css(),
  html: async () => (await import('@codemirror/lang-html')).html(),
  htm: async () => (await import('@codemirror/lang-html')).html(),
  md: async () => (await import('@codemirror/lang-markdown')).markdown(),
  markdown: async () => (await import('@codemirror/lang-markdown')).markdown(),
  yaml: async () => (await import('@codemirror/lang-yaml')).yaml(),
  yml: async () => (await import('@codemirror/lang-yaml')).yaml(),
  sql: async () => (await import('@codemirror/lang-sql')).sql()
}

/** The language support for a path, loaded on first use; null for plain text. */
export async function editorLanguage(path: string): Promise<Extension | null> {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  // `.blade.php` is PHP enough to colour.
  const extension = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
  // Own keys only: `notes.constructor` or `x.toString` would otherwise find
  // Object's own methods and hand the editor something that is no language.
  const load = Object.prototype.hasOwnProperty.call(LANGUAGES, extension) ? LANGUAGES[extension] : undefined
  if (!load) return null
  try {
    return await load()
  } catch {
    return null
  }
}

// --- colour --------------------------------------------------------------------

/** Each lezer tag, and the TextMate scopes a theme colours it under, best first. */
const SCOPES: [Tag | Tag[], string[]][] = [
  [[t.comment, t.lineComment, t.blockComment, t.docComment], ['comment']],
  [[t.controlKeyword, t.moduleKeyword], ['keyword.control', 'keyword']],
  [[t.keyword, t.definitionKeyword, t.modifier, t.self], ['storage.type', 'keyword', 'storage']],
  [t.operatorKeyword, ['keyword.operator', 'keyword']],
  [[t.operator, t.compareOperator, t.arithmeticOperator, t.logicOperator], ['keyword.operator']],
  [[t.string, t.special(t.string), t.docString], ['string']],
  [t.regexp, ['string.regexp', 'string']],
  [t.escape, ['constant.character.escape', 'constant.character']],
  [[t.number, t.integer, t.float], ['constant.numeric', 'constant']],
  [[t.bool, t.null, t.atom], ['constant.language', 'constant']],
  [[t.function(t.variableName), t.function(t.propertyName)], ['entity.name.function', 'support.function']],
  [t.definition(t.function(t.variableName)), ['entity.name.function']],
  [[t.typeName, t.className, t.namespace], ['entity.name.type', 'support.type', 'entity.name.class', 'support.class']],
  [t.propertyName, ['variable.other.property', 'support.type.property-name', 'variable']],
  [t.definition(t.variableName), ['variable.other', 'variable']],
  [t.variableName, ['variable.other', 'variable']],
  [t.special(t.variableName), ['variable.language', 'variable']],
  [t.tagName, ['entity.name.tag']],
  [t.attributeName, ['entity.other.attribute-name']],
  [t.attributeValue, ['string']],
  [t.heading, ['markup.heading', 'entity.name.section']],
  [[t.link, t.url], ['markup.underline.link', 'string']],
  [t.emphasis, ['markup.italic']],
  [t.strong, ['markup.bold']],
  [t.meta, ['meta.preprocessor', 'meta']],
  [t.invalid, ['invalid']]
]

/** The colour a theme gives a scope: the rule naming the most of it wins. */
function colourOf(rules: { scopes: string[]; color?: string; fontStyle?: string }[], wanted: string) {
  let best: { length: number; color?: string; fontStyle?: string } | null = null
  for (const rule of rules) {
    if (!rule.color && !rule.fontStyle) continue
    for (const scope of rule.scopes) {
      // A selector with a space in it is about nesting, which a flat tag
      // cannot be asked.
      if (scope.includes(' ')) continue
      if (wanted === scope || wanted.startsWith(`${scope}.`)) {
        if (!best || scope.length > best.length) {
          best = { length: scope.length, color: rule.color, fontStyle: rule.fontStyle }
        }
      }
    }
  }
  return best
}

/** One tag group and how the theme paints it. */
export interface SyntaxSpec {
  tag: Tag | Tag[]
  color?: string
  fontStyle?: string
  fontWeight?: string
}

/** The syntax colours as specs, from the Shiki theme the diff is painted in. */
export function syntaxSpecs(): SyntaxSpec[] {
  const theme = themeRules()
  if (!theme) return []
  const specs: SyntaxSpec[] = []
  for (const [tags, scopes] of SCOPES) {
    for (const scope of scopes) {
      const found = colourOf(theme.rules, scope)
      if (!found) continue
      specs.push({
        tag: tags,
        color: found.color,
        fontStyle: found.fontStyle?.includes('italic') ? 'italic' : undefined,
        fontWeight: found.fontStyle?.includes('bold') ? 'bold' : undefined
      })
      break
    }
  }
  return specs
}

/** The syntax colours, as the editor and the preview use them. */
export function syntaxColours(): Extension {
  const specs = syntaxSpecs()
  return specs.length ? syntaxHighlighting(HighlightStyle.define(specs)) : []
}

/**
 * The parser for a line or two cut out of a file, for colouring search
 * results. PHP is read as plain PHP (a lone line has no `<?php` before it),
 * and a Vue file as TypeScript, which is what a matched line in one usually
 * is. Null for a file with no language.
 */
export async function snippetLanguage(path: string): Promise<Language | null> {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  const extension = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
  try {
    if (extension === 'php') return (await import('@codemirror/lang-php')).php({ plain: true }).language
    if (extension === 'vue') {
      return (await import('@codemirror/lang-javascript')).javascript({ typescript: true }).language
    }
    const load = LANGUAGES[extension]
    if (!load) return null
    const support = (await load()) as { language?: Language }
    return support.language ?? null
  } catch {
    return null
  }
}

/** The chrome, from the app's own variables. */
export const chrome = EditorView.theme({
  '&': {
    height: '100%',
    color: 'var(--text)',
    backgroundColor: 'transparent',
    // The code font, which ⌘+ and ⌘− over code change (useCodeFont.ts).
    fontSize: 'var(--code-size, 12px)'
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': {
    fontFamily: 'var(--mono)',
    lineHeight: 'var(--code-row, 18px)'
  },
  '.cm-content': { padding: '4px 0 24px', caretColor: 'var(--text)' },
  '.cm-line': { padding: '0 12px 0 6px' },
  '.cm-gutters': {
    backgroundColor: 'var(--bg)',
    color: 'var(--text-faint)',
    border: 'none'
  },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 6px 0 12px', minWidth: '36px' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--hover) 55%, transparent)' },
  '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--text-dim)' },
  '&.cm-focused .cm-cursor': { borderLeftColor: 'var(--text)' },
  // The accent, mixed strong enough to see on every theme's ground (light and
  // dark: at least 1.5:1 against --bg focused), and weaker when the editor is
  // not focused. --primary-bg, the tint of a selected row, nearly vanished on
  // dark themes. CodeMirror's own rules are more specific, hence !important.
  '.cm-selectionBackground': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 20%, transparent) !important'
  },
  '&.cm-focused .cm-selectionBackground': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 35%, transparent) !important'
  },
  // The active line is painted over the selection layer: its tint would wash
  // a selection on that line out, so it steps aside while text is selected.
  '&.cm-hasSelection .cm-activeLine': { backgroundColor: 'transparent' },
  '.cm-matchingBracket': {
    backgroundColor: 'var(--active)',
    outline: '1px solid var(--line)'
  },
  // The change bars, as the file view drew them and the change ruler still
  // does: green where a line is new, the accent where one was rewritten, a
  // grey dot on the seam where lines were deleted. A run of lines is one bar,
  // square where it meets the next line and rounded only at its ends, and
  // hovering any line of it lights the whole run.
  '.cm-bars': { width: '7px' },
  '.cm-bars .cm-gutterElement': { position: 'relative', overflow: 'visible' },
  // Only where there is a bar to click.
  '.cm-bars .cm-gutterElement:has(.cm-bar)': { cursor: 'pointer' },
  '.cm-bar': { position: 'absolute', top: '0', bottom: '0', left: '2px', width: '3px' },
  '.cm-bar.head': { borderTopLeftRadius: '2px', borderTopRightRadius: '2px' },
  '.cm-bar.tail': { borderBottomLeftRadius: '2px', borderBottomRightRadius: '2px' },
  '.cm-bar.added': { backgroundColor: 'var(--green)' },
  '.cm-bar.changed': { backgroundColor: 'var(--accent)' },
  '.cm-bar.lit': { filter: 'brightness(1.35)' },
  '.cm-bar.removed': {
    top: '-3px',
    bottom: 'auto',
    left: '1px',
    width: '5px',
    height: '5px',
    borderRadius: '50%',
    backgroundColor: 'var(--text-faint)',
    zIndex: '1'
  },
  '.cm-bar.removed:hover, .cm-bar.removed.lit': { backgroundColor: 'var(--text-dim)', filter: 'none' },
  // The blame column: there only while it is on, a fixed width that grows
  // with the code font, and quiet enough that the code stays what is read.
  // CodeMirror's own `.cm-gutter { display: flex !important }` wins over a
  // `display: none`, so the column is closed by giving it no width instead.
  '.cm-blameGutter': {
    width: '0',
    overflow: 'hidden',
    fontFamily: 'var(--font)',
    fontSize: 'calc(var(--code-size, 12px) - 1px)'
  },
  '&.cm-blame-on .cm-blameGutter': {
    width: 'calc(var(--code-size, 12px) * 17)',
    borderRight: '1px solid var(--line-soft)'
  },
  '.cm-blameGutter .cm-gutterElement': { padding: '0' },
  // A bar in the author's colour down each run and a wash of it behind, both
  // faded with age: the newest work in the file stands out, the oldest sinks
  // into the page. Mixed into transparent, so it reads on light and dark.
  '.cm-blame-entry': {
    display: 'flex',
    alignItems: 'center',
    gap: '8px',
    height: '100%',
    padding: '0 8px 0 9px',
    color: 'var(--text-dim)',
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    cursor: 'pointer',
    boxShadow: 'inset 3px 0 0 var(--blame-tint, transparent)',
    backgroundColor: 'color-mix(in srgb, var(--blame-tint, transparent) 9%, transparent)',
    opacity: 'calc(0.45 + var(--blame-age, 1) * 0.55)'
  },
  '.cm-blame-entry.head': {
    boxShadow: 'inset 3px 0 0 var(--blame-tint, transparent), inset 0 1px 0 var(--line-soft)'
  },
  '.cm-blame-entry:hover': { color: 'var(--text)' },
  '.cm-blame-entry.fresh': {
    cursor: 'default',
    color: 'var(--text-faint)',
    fontStyle: 'italic',
    boxShadow: 'inset 3px 0 0 var(--line)',
    backgroundColor: 'transparent'
  },
  '.cm-blame-date': { flex: 'none', fontVariantNumeric: 'tabular-nums', color: 'var(--text-faint)' },
  '.cm-blame-who': { flex: '1', minWidth: '0', overflow: 'hidden', textOverflow: 'ellipsis' },
  'mark.find-hit': {
    backgroundColor: 'var(--warning-bg)',
    color: 'inherit',
    borderRadius: '2px',
    outline: '1px solid var(--warning-line)'
  },
  'mark.find-hit.now': { backgroundColor: 'var(--warning)', color: 'var(--bg)' },
  '.cm-landing': { backgroundColor: 'var(--primary-bg)' },
  // ⌘ held: the name under the mouse is a link.
  '.cm-navlink': { textDecoration: 'underline', cursor: 'pointer', color: 'var(--primary)' },
  // Nothing uses it. Private code and locals fade; public code gets a hint.
  '.cm-unused': { opacity: '0.5' },
  '.cm-unused-hint': { textDecoration: 'underline dotted var(--text-faint)', textUnderlineOffset: '3px' },
  '.cm-badge': {
    marginLeft: '12px',
    padding: '0 6px',
    borderRadius: '8px',
    fontFamily: 'var(--font)',
    fontSize: '10px',
    color: 'var(--warning)',
    border: '1px solid var(--warning-line)',
    whiteSpace: 'nowrap'
  },
  '.cm-lens': {
    fontFamily: 'var(--font)',
    fontSize: 'calc(var(--code-size, 12px) - 2px)',
    lineHeight: 'calc(var(--code-row, 18px) - 4px)',
    color: 'var(--text-faint)',
    paddingLeft: '6px',
    cursor: 'pointer',
    userSelect: 'none'
  },
  '.cm-lens:hover': { color: 'var(--text-dim)', textDecoration: 'underline' },
  '.cm-lens-possible': { opacity: '0.6' }
})

// --- the change bars ------------------------------------------------------------

export interface Bar {
  line: number
  kind: 'added' | 'changed' | 'removed'
  hunk: number
}

const BAR_TITLES: Record<Bar['kind'], string> = {
  added: 'New since the last commit — click for the change',
  changed: 'Changed — click to see what was here',
  removed: 'Lines deleted here — click to read them'
}

class BarMarker extends GutterMarker {
  constructor(
    readonly kind: Bar['kind'],
    readonly hunk: number,
    /** The first and last line of a run of the same kind: rounded there. */
    readonly head: boolean,
    readonly tail: boolean
  ) {
    super()
  }

  override eq(other: BarMarker) {
    return (
      other.kind === this.kind && other.hunk === this.hunk && other.head === this.head && other.tail === this.tail
    )
  }

  override toDOM() {
    const element = document.createElement('div')
    element.className = `cm-bar ${this.kind}${this.head ? ' head' : ''}${this.tail ? ' tail' : ''}`
    element.dataset.hunk = String(this.hunk)
    element.title = BAR_TITLES[this.kind]
    return element
  }
}

/** Which hunk each line of the new file falls in, read off the headers. */
function hunkOf(hunks: DiffHunk[]) {
  const spans = hunks.map((hunk) => {
    const found = /\+(\d+)(?:,(\d+))?/.exec(hunk.header)
    const start = Number(found?.[1] ?? 0)
    const count = found?.[2] === undefined ? 1 : Number(found[2])
    return count === 0 ? { from: start + 1, to: start + 1 } : { from: start, to: start + count }
  })
  return (line: number) => {
    const at = spans.findIndex((span) => line >= span.from && line <= span.to)
    return at < 0 ? 0 : at
  }
}

/** The bars for a file's text against its hunks: the model ChangeRuler uses. */
export function barsFor(text: string, hunks: DiffHunk[]): Bar[] {
  const which = hunkOf(hunks)
  const out: Bar[] = []
  for (const line of markedLines(text, hunks)) {
    if (line.removed.length) out.push({ line: line.number, kind: 'removed', hunk: which(line.number) })
    if (line.mark) {
      out.push({ line: line.number, kind: line.mark === 'added' ? 'added' : 'changed', hunk: which(line.number) })
    }
  }
  return out
}

export const setBars = StateEffect.define<Bar[]>()

const barField = StateField.define<RangeSet<BarMarker>>({
  create: () => RangeSet.empty,
  update(value, tr) {
    let next = value.map(tr.changes)
    for (const effect of tr.effects) {
      if (!effect.is(setBars)) continue
      const doc = tr.state.doc
      const kept = effect.value
        .filter((bar) => bar.line >= 1 && bar.line <= doc.lines)
        // A line that both lost lines above it and changed shows the change;
        // the dot is for a seam with nothing else on it.
        .filter(
          (bar, _, all) =>
            bar.kind !== 'removed' || !all.some((one) => one.line === bar.line && one.kind !== 'removed')
        )
      // Which kind of bar each line has, to tell where a run starts and ends.
      const solid = new Map<number, Bar['kind']>()
      for (const bar of kept) if (bar.kind !== 'removed') solid.set(bar.line, bar.kind)
      const ranges = kept
        .map((bar) =>
          new BarMarker(
            bar.kind,
            bar.hunk,
            bar.kind === 'removed' || solid.get(bar.line - 1) !== bar.kind,
            bar.kind === 'removed' || solid.get(bar.line + 1) !== bar.kind
          ).range(doc.line(bar.line).from)
        )
        .sort((a, b) => a.from - b.from)
      next = RangeSet.of(ranges)
    }
    return next
  }
})

/** Lights every bar of one hunk, or none. */
function lightRun(root: HTMLElement, hunk: number | null) {
  for (const bar of root.querySelectorAll<HTMLElement>('.cm-bar.lit')) {
    if (bar.dataset.hunk !== String(hunk)) bar.classList.remove('lit')
  }
  if (hunk === null) return
  for (const bar of root.querySelectorAll<HTMLElement>(`.cm-bar[data-hunk="${hunk}"]`)) bar.classList.add('lit')
}

/** The gutter of change bars. `open` is called with the line of the bar clicked. */
export function changeBars(open: (line: number) => void): Extension {
  return [
    barField,
    gutter({
      class: 'cm-bars',
      markers: (view) => view.state.field(barField),
      domEventHandlers: {
        mousedown(view, block) {
          let found = false
          view.state.field(barField).between(block.from, block.from, () => {
            found = true
          })
          if (!found) return false
          open(view.state.doc.lineAt(block.from).number)
          return true
        },
        // The whole run lights up under the pointer, so it reads as one change.
        mouseover(view, block) {
          let hunk: number | null = null
          view.state.field(barField).between(block.from, block.from, (_from, _to, marker) => {
            hunk = marker.hunk
          })
          lightRun(view.dom, hunk)
          return false
        },
        mouseleave(view) {
          lightRun(view.dom, null)
          return false
        }
      }
    })
  ]
}

// --- who last touched each line, in a column left of the numbers ---------------------

/**
 * The runs to annotate the file with, null to close the column.
 *
 * `text` is the file the runs were read from (the copy on disk). When the
 * document has moved on from it by the time they arrive, the lines that differ
 * are not committed whatever the runs say, and are marked so.
 */
export const setBlame = StateEffect.define<{ runs: BlameRun[]; text?: string | null } | null>()

/** One line's answer: the run it came in with, or null for a line typed since. */
class LineBlame extends RangeValue {
  override point = true
  constructor(readonly run: BlameRun | null) {
    super()
  }
  override eq(other: LineBlame) {
    return other.run === this.run
  }
}

const TYPED = new LineBlame(null)

interface BlameState {
  /** One value per line, at the line's start, so it moves with the text. */
  lines: RangeSet<LineBlame>
  oldest: number
  span: number
}

/** The run each line of `text` belongs to, by line number. */
function linesOf(runs: BlameRun[]) {
  const out: BlameRun[] = []
  for (const run of runs) for (let at = 0; at < run.lines; at++) out[run.start - 1 + at] = run
  return out
}

function buildBlame(doc: Text, runs: BlameRun[], text: string | null | undefined): BlameState {
  const byLine = linesOf(runs)
  // The document against the text the runs describe: the lines they share at
  // the top and the bottom keep their answer, the ones between were typed.
  // A run, null for a line typed since, undefined for a line with no answer.
  let map = (line: number): BlameRun | null | undefined => byLine[line - 1]
  if (text != null && text !== doc.toString()) {
    const old = text.split('\n')
    let top = 0
    while (top < old.length && top < doc.lines && old[top] === doc.line(top + 1).text) top++
    let bottom = 0
    while (
      bottom < old.length - top &&
      bottom < doc.lines - top &&
      old[old.length - 1 - bottom] === doc.line(doc.lines - bottom).text
    ) {
      bottom++
    }
    const shift = old.length - doc.lines
    map = (line) => {
      if (line <= top) return byLine[line - 1]
      if (line > doc.lines - bottom) return byLine[line - 1 + shift]
      return null
    }
  }
  const builder = new RangeSetBuilder<LineBlame>()
  for (let number = 1; number <= doc.lines; number++) {
    const run = map(number)
    const line = doc.line(number)
    // No answer for the empty line after a file's last newline is no mark: it
    // is not a line git counts. Any other line without one is newer than the
    // answer (lines added since it was read), so it is not committed yet
    // rather than a blank in the column.
    if (run === undefined && number === doc.lines && !line.length) continue
    builder.add(line.from, line.from, run ? new LineBlame(run) : TYPED)
  }
  const times = runs.filter((run) => !run.uncommitted).map((run) => run.time)
  const newest = times.length ? Math.max(...times) : 0
  const oldest = times.length ? Math.min(...times) : 0
  return { lines: builder.finish(), oldest, span: newest - oldest }
}

/** The answer for the line starting at `from`. */
function blameAt(set: RangeSet<LineBlame>, from: number): LineBlame | null {
  let found: LineBlame | null = null
  set.between(from, from, (start, _to, value) => {
    if (start !== from) return
    found = value
    return false
  })
  return found
}

/**
 * The answers carried through an edit.
 *
 * Lines outside what changed keep theirs, moved with the text. Inside it, a
 * line still reading as the line it was at either edge of the change (Enter at
 * the end of a line, a line pasted above another) keeps its answer, and every
 * other line is new work, not committed yet: no line ever shows the author of
 * the line that used to be there.
 */
function mapBlame(value: BlameState, tr: Transaction): BlameState {
  const before = tr.startState.doc
  const after = tr.state.doc
  let lines = value.lines.map(tr.changes)
  tr.changes.iterChangedRanges((fromA, toA, fromB, toB) => {
    const a1 = before.lineAt(fromA).number
    const a2 = before.lineAt(toA).number
    const b1 = after.lineAt(fromB).number
    const b2 = after.lineAt(toB).number
    const was = (line: number) => blameAt(value.lines, before.line(line).from)
    const kept = new Map<number, LineBlame | null>()
    let lo = b1
    let hi = b2
    let oldLo = a1
    let oldHi = a2
    while (lo <= hi && oldLo <= oldHi && after.line(lo).text === before.line(oldLo).text) {
      kept.set(lo++, was(oldLo++))
    }
    while (hi >= lo && oldHi >= oldLo && after.line(hi).text === before.line(oldHi).text) {
      kept.set(hi--, was(oldHi--))
    }
    const from = after.line(b1).from
    const to = after.line(b2).to
    const add: Range<LineBlame>[] = []
    for (let number = b1; number <= b2; number++) {
      const mark = kept.has(number) ? kept.get(number) : TYPED
      if (mark) add.push(mark.range(after.line(number).from))
    }
    lines = lines.update({
      filterFrom: from,
      filterTo: to,
      filter: (start) => start < from || start > to,
      add
    })
  })
  return { ...value, lines }
}

/** The lines git counts in a text: none for an empty one, no line after the last newline. */
function countedLines(text: string) {
  if (!text) return 0
  return text.split('\n').length - (text.endsWith('\n') ? 1 : 0)
}

/**
 * Whether the runs can be the answer for `text`. Blame is read in the
 * background and the file is saved as it is typed, so an answer can arrive
 * for a copy of the file that has been saved over since: one with another
 * number of lines is that, and laid over this text it would put every line
 * under the change down to the wrong commit.
 */
function answers(runs: BlameRun[], text: string) {
  return runs.reduce((sum, run) => sum + run.lines, 0) === countedLines(text)
}

const blameField = StateField.define<BlameState | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setBlame)) {
        const asked = effect.value
        if (!asked) return null
        if (asked.text != null && !answers(asked.runs, asked.text)) {
          // The column there is has followed every edit since: it stays until
          // the answer for the file as saved comes in. With none yet, the runs
          // are laid over the lines as they stand.
          return value ? (tr.docChanged ? mapBlame(value, tr) : value) : buildBlame(tr.state.doc, asked.runs, null)
        }
        return buildBlame(tr.state.doc, asked.runs, asked.text)
      }
    }
    return value && tr.docChanged ? mapBlame(value, tr) : value
  }
})

/** The run a line was blamed on, null for none or one typed since. */
export function blameOfLine(state: EditorState, line: number): BlameRun | null {
  const field = state.field(blameField, false)
  if (!field || line < 1 || line > state.doc.lines) return null
  return blameAt(field.lines, state.doc.line(line).from)?.run ?? null
}

/** Whether the column is open. */
export function blameShown(state: EditorState) {
  return !!state.field(blameField, false)
}

/** Lines typed since and lines blame calls uncommitted read as one change. */
const groupOf = (blame: LineBlame | null) => {
  if (!blame) return null
  return !blame.run || blame.run.uncommitted ? 'new' : blame.run
}

class BlameMarker extends GutterMarker {
  constructor(
    readonly run: BlameRun | null,
    readonly head: boolean,
    readonly age: number
  ) {
    super()
  }

  override eq(other: BlameMarker) {
    return other.run === this.run && other.head === this.head && other.age === this.age
  }

  override toDOM() {
    const element = document.createElement('div')
    const fresh = !this.run || this.run.uncommitted
    element.className = `cm-blame-entry${this.head ? ' head' : ''}${fresh ? ' fresh' : ''}`
    element.title = blameTooltip(this.run)
    if (!fresh) {
      element.style.setProperty('--blame-tint', tint(this.run!.email || this.run!.author))
      element.style.setProperty('--blame-age', String(this.age))
    }
    if (this.head) {
      if (fresh) {
        const note = document.createElement('span')
        note.className = 'cm-blame-who'
        note.textContent = 'Not committed yet'
        element.append(note)
      } else {
        const date = document.createElement('span')
        date.className = 'cm-blame-date'
        date.textContent = blameDate(this.run!.time)
        const who = document.createElement('span')
        who.className = 'cm-blame-who'
        who.textContent = this.run!.author
        element.append(date, who)
      }
    }
    return element
  }
}

const blameOn = EditorView.editorAttributes.compute([blameField], (state): Record<string, string> =>
  state.field(blameField) ? { class: 'cm-blame-on' } : {}
)

/**
 * The blame column, the way an IDE annotates a file: the date and the author
 * on the first line of each run of lines from one commit, a bar in the
 * author's colour down the run, and newer commits shaded stronger than old
 * ones. Markers are made for the lines on screen only, read off a range set
 * that moves with the text, so a file of twenty thousand lines costs a
 * screenful of them.
 *
 * `open` is called with the run of a clicked entry and the entry's element,
 * which the popover it opens hangs from. A line not committed yet has no
 * commit to open, and a click on it does nothing.
 */
export function blameGutter(open: (run: BlameRun, entry: HTMLElement | null) => void): Extension {
  return [
    blameField,
    blameOn,
    Prec.high(
      gutter({
        class: 'cm-blameGutter',
        lineMarker(view, line) {
          const field = view.state.field(blameField)
          if (!field) return null
          const here = blameAt(field.lines, line.from)
          if (!here) return null
          const number = view.state.doc.lineAt(line.from).number
          const above = number > 1 ? blameAt(field.lines, view.state.doc.line(number - 1).from) : null
          const head = number === 1 || groupOf(above) !== groupOf(here)
          const run = here.run
          const age =
            run && !run.uncommitted && field.span > 0 ? (run.time - field.oldest) / field.span : 1
          return new BlameMarker(run, head, Math.round(age * 20) / 20)
        },
        lineMarkerChange: (update) =>
          update.startState.field(blameField) !== update.state.field(blameField),
        domEventHandlers: {
          mousedown(view, line, event) {
            if ((event as MouseEvent).button !== 0) return false
            const run = blameAt(view.state.field(blameField)?.lines ?? RangeSet.empty, line.from)?.run
            if (!run || run.uncommitted) return false
            const target = event.target as Element | null
            open(run, (target?.closest?.('.cm-gutterElement') as HTMLElement | null) ?? null)
            return true
          }
        }
      })
    )
  ]
}

// --- the find box's matches ---------------------------------------------------------

export const setFind = StateEffect.define<FindMarks | null>()

const findState = StateField.define<FindMarks | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setFind)) return effect.value
    return value
  }
})

const hitMark = Decoration.mark({ tagName: 'mark', class: 'find-hit' })
const nowMark = Decoration.mark({ tagName: 'mark', class: 'find-hit now' })

function findDecorations(view: EditorView): DecorationSet {
  const find = view.state.field(findState)
  if (!find?.query) return Decoration.none
  const marks: Range<Decoration>[] = []
  for (const { from, to } of view.visibleRanges) {
    let line = view.state.doc.lineAt(from)
    for (;;) {
      const row = line.number - 1
      occurrences(line.text, find.query, find.matchCase).forEach(([start, end], nth) => {
        const mark = find.row === row && find.nth === nth ? nowMark : hitMark
        marks.push(mark.range(line.from + start, line.from + end))
      })
      if (line.to >= to || line.number >= view.state.doc.lines) break
      line = view.state.doc.line(line.number + 1)
    }
  }
  return Decoration.set(marks, true)
}

const findMarks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    constructor(view: EditorView) {
      this.decorations = findDecorations(view)
    }
    update(update: ViewUpdate) {
      const changed = update.transactions.some((tr) => tr.effects.some((e) => e.is(setFind)))
      if (changed || update.docChanged || update.viewportChanged) {
        this.decorations = findDecorations(update.view)
      }
    }
  },
  { decorations: (plugin) => plugin.decorations }
)

export const findHighlight: Extension = [findState, findMarks]

/** Where the find box's current match is in the document, if it is in it. */
export function findPosition(view: EditorView, find: FindMarks | null) {
  if (!find?.query || find.row === null) return null
  if (find.row + 1 > view.state.doc.lines) return null
  const line = view.state.doc.line(find.row + 1)
  const hit = occurrences(line.text, find.query, find.matchCase)[find.nth]
  return hit ? { from: line.from + hit[0], to: line.from + hit[1] } : null
}

// --- every match of a search, for its preview ---------------------------------------

export interface LineHits {
  line: number
  /** UTF-16 offsets in the line. */
  ranges: [number, number][]
}

/** The matches to mark, and the line of the one selected. */
export const setHits = StateEffect.define<{ hits: LineHits[]; now: number | null }>()

const hitsField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let next = value.map(tr.changes)
    for (const effect of tr.effects) {
      if (!effect.is(setHits)) continue
      const doc = tr.state.doc
      const marks: Range<Decoration>[] = []
      for (const { line, ranges } of effect.value.hits) {
        if (line < 1 || line > doc.lines) continue
        const at = doc.line(line)
        const mark = line === effect.value.now ? nowMark : hitMark
        for (const [from, to] of ranges) {
          const start = Math.min(from, at.length)
          const end = Math.min(to, at.length)
          if (end > start) marks.push(mark.range(at.from + start, at.from + end))
        }
      }
      next = Decoration.set(marks, true)
    }
    return next
  },
  provide: (field) => EditorView.decorations.from(field)
})

export const searchHits: Extension = hitsField

// --- a line picked out, for a jump and the search preview --------------------------

export const setLanding = StateEffect.define<number | null>()

const landingLine = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let next = value.map(tr.changes)
    for (const effect of tr.effects) {
      if (!effect.is(setLanding)) continue
      const number = effect.value
      next =
        number && number >= 1 && number <= tr.state.doc.lines
          ? Decoration.set([Decoration.line({ class: 'cm-landing' }).range(tr.state.doc.line(number).from)])
          : Decoration.none
    }
    return next
  },
  provide: (field) => EditorView.decorations.from(field)
})

export const landing: Extension = landingLine

// --- keys -----------------------------------------------------------------------------

/**
 * CodeMirror's own keys, less the ones the app has given to something else:
 * ⌥↑/↓ (move line) are next and previous change here, and ⌘[ / ⌘] (indent)
 * are back and forward.
 */
/**
 * Keys the app has for itself, which the editor's own keymap would otherwise
 * take: next and previous change (⌥↑/↓, CodeMirror's move line), back and
 * forward (⌘[ / ⌘], and ⌥← / ⌥→ off macOS), next and previous changed file
 * (⌘⌥↑/↓). CodeMirror prevents the default of what it handles, and the
 * app's dispatcher leaves alone anything already handled.
 */
const isMacKeys = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')
const TAKEN = new Set([
  'Alt-ArrowUp',
  'Alt-ArrowDown',
  'Mod-[',
  'Mod-]',
  'Mod-Alt-ArrowUp',
  'Mod-Alt-ArrowDown',
  // On a Mac ⌥← and ⌥→ move by word, as everywhere else on a Mac.
  ...(isMacKeys ? [] : ['Alt-ArrowLeft', 'Alt-ArrowRight'])
])

export function editorKeys(own: KeyBinding[]): Extension {
  // By the key a binding has on this platform: Mod-← is word-left off a Mac
  // and ⌥← is word-left on one, and neither is taken.
  const kept = defaultKeymap.filter(
    (binding) => !TAKEN.has((isMacKeys ? (binding.mac ?? binding.key) : binding.key) ?? '')
  )
  return keymap.of([...own, indentWithTab, ...historyKeymap, ...kept])
}

/** `cm-hasSelection` on the editor while any text is selected. */
const hasSelection = EditorView.editorAttributes.compute(['selection'], (state): Record<string, string> =>
  state.selection.ranges.some((range) => !range.empty) ? { class: 'cm-hasSelection' } : {}
)

/** Everything a view starts with apart from its document, language and keys. */
export function basics(): Extension {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightActiveLine(),
    drawSelection(),
    hasSelection,
    bracketMatching(),
    chrome,
    findHighlight,
    landing
  ]
}

export function editingBasics(): Extension {
  return [history(), indentOnInput()]
}

/** A slot whose content can be swapped without rebuilding the view. */
export function slot() {
  return new Compartment()
}

/** Pick the readonly flags in one place. */
export function readOnly(on: boolean): Extension {
  return [EditorView.editable.of(!on), EditorView.contentAttributes.of({ 'aria-readonly': String(on) })]
}

// --- the view the user last typed in, for the search window ------------------------

let focused: EditorView | null = null

export function rememberFocus(view: EditorView | null) {
  focused = view
}

/** The text selected in the editor that had the caret last, if it is one line. */
export function editorSelection(): string {
  const view = focused
  if (!view || !view.dom.isConnected) return ''
  const { from, to } = view.state.selection.main
  const text = view.state.sliceDoc(from, to)
  return text && !text.includes('\n') && text.length <= 200 ? text : ''
}

/** The lines the change bars sit on, in order: where next and previous change go. */
export function barLines(view: EditorView): number[] {
  const lines: number[] = []
  let previous = -1
  view.state.field(barField).between(0, view.state.doc.length, (from) => {
    const number = view.state.doc.lineAt(from).number
    // A run of marked lines is one change: only where a run starts counts.
    if (number !== previous + 1 && number !== previous) lines.push(number)
    previous = number
  })
  return lines
}

// --- ⌘-click and the names under the mouse ------------------------------------------

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')

/** ⌘ on a Mac, Ctrl elsewhere: the key that turns names into links. */
export function navKey(event: MouseEvent | KeyboardEvent) {
  return isMac ? event.metaKey : event.ctrlKey
}

const linkMark = Decoration.mark({ class: 'cm-navlink' })

/**
 * Holding ⌘ underlines the word under the mouse; ⌘-click calls `open` with
 * the position clicked. What the word is, and where it leads, is the
 * backend's to say.
 */
export function navigation(
  open: (pos: number, event: MouseEvent) => void,
  /** A wider token than the word at `pos` (a framework's `Foo\\Bar`), if any. */
  token?: (state: EditorState, pos: number) => { from: number; to: number } | null
): Extension {
  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none
      held = false
      /** Whether the pointer is over the editor: only then is there a word under it. */
      inside = false
      x = 0
      y = 0
      /** The word underlined now, as `from:to`, or empty. */
      shown = ''
      constructor(readonly view: EditorView) {}
      refresh() {
        const view = this.view
        let word: { from: number; to: number } | null = null
        if (this.held) {
          const pos = view.posAtCoords({ x: this.x, y: this.y })
          word = pos === null ? null : (token?.(view.state, pos) ?? view.state.wordAt(pos))
        }
        const key = word ? `${word.from}:${word.to}` : ''
        if (key === this.shown) return
        this.shown = key
        this.decorations = word ? Decoration.set([linkMark.range(word.from, word.to)]) : Decoration.none
        view.dispatch({})
      }
      update(update: ViewUpdate) {
        if (update.docChanged && this.shown) {
          this.shown = ''
          this.decorations = Decoration.none
        }
      }
    },
    {
      decorations: (plugin) => plugin.decorations,
      eventHandlers: {
        mousemove(event: MouseEvent) {
          this.inside = true
          this.x = event.clientX
          this.y = event.clientY
          const held = navKey(event)
          if (held || this.held) {
            this.held = held
            this.refresh()
          }
        },
        blur() {
          // ⌘-tab away: the key comes up somewhere else and is never heard.
          if (this.held) {
            this.held = false
            this.refresh()
          }
        },
        mouseleave() {
          this.inside = false
          if (this.held) {
            this.held = false
            this.refresh()
          }
        },
        keydown(event: KeyboardEvent) {
          if ((event.key === 'Meta' || event.key === 'Control') && !this.held && this.inside) {
            this.held = true
            this.refresh()
          }
        },
        keyup(event: KeyboardEvent) {
          if ((event.key === 'Meta' || event.key === 'Control') && this.held) {
            this.held = false
            this.refresh()
          }
        },
        mousedown(event: MouseEvent, view: EditorView) {
          if (event.button !== 0 || !navKey(event) || event.altKey || event.shiftKey) return false
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY })
          if (pos === null) return false
          event.preventDefault()
          open(pos, event)
          return true
        }
      }
    }
  )
  return plugin
}

// --- unused code and usage counts ----------------------------------------------------

/** One thing nothing uses, as `unused_in` gives it. */
export interface UnusedMark {
  name: string
  kind: 'function' | 'method' | 'variable'
  level: 'dim' | 'hint'
  line: number
  from: number
  to: number
  block_from: number
  block_to: number
  new: boolean
}

/** "3 usages" over one definition, as `usage_counts` gives it. */
export interface UsageLens {
  name: string
  kind: string
  line: number
  from: number
  /** Usages on the definition's class or one in its hierarchy. */
  count: number
  /** Calls by the name on something whose class is not known. */
  possible?: number
  /** The framework calls it, and `count` is the uses of what it belongs to. */
  framework?: boolean
  /** The class whose uses `count` is. */
  owner?: string | null
  /** Places in a framework's config that name the class. */
  config?: number
}

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`

/**
 * What a lens says: "2 usages", with "· 50 possible" kept apart to be
 * shown muted, or for what the framework calls, "2 uses of ExportCommand",
 * "called by the framework · 2 references", or "called by the framework".
 */
export function lensText(lens: UsageLens): { main: string; possible: string } {
  if (lens.framework) {
    if (!lens.count) return { main: 'called by the framework', possible: '' }
    if (lens.owner) return { main: `${plural(lens.count, 'use', 'uses')} of ${lens.owner}`, possible: '' }
    return { main: `called by the framework · ${plural(lens.count, 'reference', 'references')}`, possible: '' }
  }
  const config = lens.config ?? 0
  if (config && !lens.count) {
    return {
      main: plural(config, 'reference in config', 'references in config'),
      possible: lens.possible ? `${lens.possible} possible` : ''
    }
  }
  return {
    main: plural(lens.count, 'usage', 'usages') + (config ? ` · ${config} in config` : ''),
    possible: lens.possible ? `${lens.possible} possible` : ''
  }
}

export const setUnused = StateEffect.define<UnusedMark[]>()
export const setLenses = StateEffect.define<UsageLens[]>()

class BadgeWidget extends WidgetType {
  override eq() {
    return true
  }

  override toDOM() {
    const span = document.createElement('span')
    span.className = 'cm-badge'
    span.textContent = 'new · 0 usages'
    return span
  }
}

const dimMark = Decoration.mark({ class: 'cm-unused' })
const hintMark = Decoration.mark({ class: 'cm-unused-hint' })

function unusedDecorations(doc: { length: number; lineAt(pos: number): { to: number } }, marks: UnusedMark[]) {
  const ranges: Range<Decoration>[] = []
  const fit = (pos: number) => Math.max(0, Math.min(pos, doc.length))
  for (const mark of marks) {
    const from = fit(mark.level === 'dim' ? mark.block_from : mark.from)
    const to = fit(mark.level === 'dim' ? mark.block_to : mark.to)
    if (to > from) ranges.push((mark.level === 'dim' ? dimMark : hintMark).range(from, to))
    if (mark.new && mark.kind !== 'variable') {
      const end = doc.lineAt(fit(mark.from)).to
      ranges.push(Decoration.widget({ widget: new BadgeWidget(), side: 1 }).range(end))
    }
  }
  return Decoration.set(ranges, true)
}

const unusedField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setUnused)) return unusedDecorations(tr.state.doc, effect.value)
    }
    return tr.docChanged ? value.map(tr.changes) : value
  },
  provide: (field) => EditorView.decorations.from(field)
})

/** Unused code, dimmed or hinted, with the badge for what is new on the branch. */
export const unusedCode: Extension = unusedField

type LensOpener = (lens: UsageLens, event: MouseEvent) => void

/**
 * What a click on a count does, looked up when it is clicked: the counts are
 * kept in a field every editor shares, so a state handed from one editor to
 * the next keeps them and calls the editor it is in now.
 */
const lensOpener = Facet.define<LensOpener, LensOpener | null>({
  combine: (all) => all[all.length - 1] ?? null
})

class LensWidget extends WidgetType {
  constructor(
    readonly lens: UsageLens,
    readonly indent: string
  ) {
    super()
  }

  override eq(other: LensWidget) {
    const a = other.lens
    const b = this.lens
    return (
      a.count === b.count &&
      a.name === b.name &&
      (a.possible ?? 0) === (b.possible ?? 0) &&
      !!a.framework === !!b.framework &&
      (a.owner ?? null) === (b.owner ?? null) &&
      (a.config ?? 0) === (b.config ?? 0) &&
      other.indent === this.indent
    )
  }

  override toDOM(view: EditorView) {
    const div = document.createElement('div')
    div.className = 'cm-lens'
    div.style.paddingLeft = `calc(6px + ${this.indent.length}ch)`
    const { main, possible } = lensText(this.lens)
    div.textContent = main
    if (possible) {
      const muted = document.createElement('span')
      muted.className = 'cm-lens-possible'
      muted.textContent = ` · ${possible}`
      div.appendChild(muted)
    }
    if (this.lens.framework) {
      div.title = this.lens.owner
        ? `The framework calls ${this.lens.name}(). Counted: the places that use ${this.lens.owner}.`
        : `The framework calls ${this.lens.name}().`
    } else if (possible) {
      div.title = 'Possible: the same name, called on something whose class could not be worked out.'
    }
    div.addEventListener('mousedown', (event) => {
      event.preventDefault()
      view.state.facet(lensOpener)?.(this.lens, event)
    })
    return div
  }

  override ignoreEvent() {
    return true
  }
}

const lensField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (!effect.is(setLenses)) continue
      const ranges: Range<Decoration>[] = []
      const seen = new Set<number>()
      for (const lens of effect.value) {
        if (lens.line < 1 || lens.line > tr.state.doc.lines || seen.has(lens.line)) continue
        seen.add(lens.line)
        const line = tr.state.doc.line(lens.line)
        const indent = /^\s*/.exec(line.text)?.[0] ?? ''
        ranges.push(
          Decoration.widget({ widget: new LensWidget(lens, indent), block: true, side: -1 }).range(line.from)
        )
      }
      return Decoration.set(ranges, true)
    }
    return tr.docChanged ? value.map(tr.changes) : value
  },
  provide: (field) => EditorView.decorations.from(field)
})

/** "3 usages" over each definition; a click on one lists them. */
export function usageLenses(open: LensOpener): Extension {
  return [lensField, lensOpener.of(open)]
}

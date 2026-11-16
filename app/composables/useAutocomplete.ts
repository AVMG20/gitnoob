import {
  acceptCompletion,
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  closeCompletion,
  completeAnyWord,
  completeFromList,
  completionStatus,
  insertCompletionText,
  moveCompletionSelection,
  pickedCompletion,
  startCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource
} from '@codemirror/autocomplete'
import { syntaxTree } from '@codemirror/language'
import { Prec, type EditorState, type Extension } from '@codemirror/state'
import { EditorView, keymap, type Command } from '@codemirror/view'
import { invokeAt } from './useInvoke'

/**
 * Autocomplete in the editor. Not an IDE: no language server and no type
 * inference past what the backend reads off the code (complete.rs, deps.rs).
 *
 * Two kinds of list. After a word of two letters or more: the language's own
 * keywords and snippets where its package has them, the words of the file,
 * and the project's classes, functions and constants from the symbol index,
 * nearest first. Straight after `->`, `::` or `.`: the members of the
 * receiver's class, when the backend can say which class that is, and
 * nothing when it cannot.
 *
 * The keys belong to the list only while it is open: ↵ and Tab accept, Esc
 * closes, ↑ and ↓ move. Those presses stop here, so the window's own Esc
 * (back to the graph, closing the find box) never hears them. With the list
 * closed, Tab indents and ↵ is a new line as before.
 */

export interface AutocompleteOptions {
  /** The repository the file is in, fixed when the editor opened. */
  repo: string | null
  path: () => string
  /** Settings → Editor, read at every keystroke. */
  settings: () => { on: boolean; typing: boolean }
  /** Changed files, which rank after the file's own folder. */
  changed: () => string[]
}

/** One member as `complete_members` gives it. */
export interface MemberItem {
  name: string
  kind: 'method' | 'property' | 'constant' | 'case'
  static: boolean
  visibility: string
  signature: string | null
  summary: string | null
  class: string
  inherited: boolean
}

export interface MemberList {
  receiver: string | null
  access: 'instance' | 'static' | 'parent' | null
  from: number
  items: MemberItem[]
}

/** One project name as `complete_symbols` gives it. */
export interface SymbolItem {
  name: string
  kind: 'class' | 'interface' | 'trait' | 'enum' | 'function' | 'constant'
  path: string
}

/** How long typing has to pause before the backend is asked. */
export const SETTLE_MS = 60
/** How many project names are asked for. */
const SYMBOL_LIMIT = 40

const MEMBER_TYPES: Record<string, string> = {
  method: 'method',
  property: 'property',
  constant: 'constant',
  case: 'case'
}

const PHP = /\.php$/i
const SCRIPT = /\.(m|c)?(j|t)sx?$|\.vue$/i
/** The languages the backend knows members in. */
const MEMBERS = /\.(php|m?[jt]sx?|c[jt]s|vue)$/i

/** Whether `pos` is inside a comment or a string (not in an interpolation in one). */
export function quiet(state: EditorState, pos: number): boolean {
  type Found = ReturnType<ReturnType<typeof syntaxTree>['resolveInner']>
  for (let node: Found | null = syntaxTree(state).resolveInner(pos, -1); node; node = node.parent) {
    const name = node.type.name
    if (/Interpolation|Substitution/.test(name)) return false
    if (!/Comment|String|Heredoc|Nowdoc|^RegExp$|^Regex/.test(name)) continue
    if (node.to !== pos) return true
    // Right after a string or a block comment that is closed: outside it.
    const text = state.sliceDoc(node.from, node.to)
    if (/Comment/.test(name)) return !text.endsWith('*/') || text.length < 4
    const quote = text[0] ?? ''
    return !(text.length >= 2 && /['"`]/.test(quote) && text.endsWith(quote))
  }
  return false
}

/** Waits for typing to pause; false when the request went stale meanwhile. */
function settle(context: CompletionContext) {
  return new Promise<boolean>((done) => setTimeout(() => done(!context.aborted), SETTLE_MS))
}

/** The `->`, `::` or `.` right before the name being typed, if there is one. */
export function memberAccess(path: string, before: string): string | null {
  if (PHP.test(path)) {
    const found = /(->|::)\$?[A-Za-z0-9_\x80-￿]*$/.exec(before)
    return found ? found[1]! : null
  }
  if (!SCRIPT.test(path)) return null
  const found = /([A-Za-z0-9_$)\]\x80-￿])\??\.[A-Za-z0-9_$\x80-￿]*$/.exec(before)
  if (!found || /^[0-9]$/.test(found[1]!)) return null
  return '.'
}

/** Whether a class picked from the list may need a `use` line. */
function importable(path: string, item: SymbolItem) {
  return PHP.test(path) && PHP.test(item.path) && item.kind !== 'function' && item.kind !== 'constant'
}

function folder(path: string) {
  const at = path.lastIndexOf('/')
  return at < 0 ? '' : path.slice(0, at)
}

export function autocomplete(options: AutocompleteOptions): Extension {
  let latestMembers = 0
  let latestNames = 0

  /** Guards every source: the setting, read-only editors, comments, strings. */
  function allowed(context: CompletionContext) {
    const { on, typing } = options.settings()
    if (!on) return false
    if (!context.explicit && !typing) return false
    if (!context.state.facet(EditorView.editable)) return false
    return !quiet(context.state, context.pos)
  }

  /** What follows `->`, `::` and `.`: the receiver's members, or nothing. */
  const members: CompletionSource = async (context) => {
    const path = options.path()
    if (!MEMBERS.test(path) || !allowed(context)) return null
    const before = context.state.sliceDoc(Math.max(0, context.pos - 200), context.pos)
    if (!memberAccess(path, before)) return null
    if (!(await settle(context))) return null
    const mine = ++latestMembers
    const list = await invokeAt<MemberList>(options.repo, 'complete_members', {
      path,
      offset: context.pos,
      text: context.state.doc.toString()
    }).catch(() => null)
    if (mine !== latestMembers || context.aborted || !list?.items.length) return null
    return {
      from: list.from,
      validFor: /^[\w$]*$/,
      options: list.items.map(
        (item): Completion => ({
          label: item.name,
          type: MEMBER_TYPES[item.kind] ?? 'property',
          detail: [item.signature ?? '', item.inherited ? `· ${item.class}` : ''].filter(Boolean).join(' '),
          info: item.summary ?? undefined,
          // Its own before what it inherits, among equal matches.
          boost: item.inherited ? -1 : 0
        })
      )
    }
  }

  /** The language's keywords and snippets, where its package has them. */
  async function fromLanguage(context: CompletionContext): Promise<CompletionResult | null> {
    const data = context.state.languageDataAt<CompletionSource | readonly (string | Completion)[]>(
      'autocomplete',
      context.pos
    )
    const sources = data.map((one) => (typeof one === 'function' ? one : completeFromList(one)))
    const results = await Promise.all(sources.map((source) => Promise.resolve(source(context)).catch(() => null)))
    const found = results.filter((one): one is CompletionResult => !!one && one.options.length > 0)
    if (!found.length) return null
    const first = found[0]!
    return {
      from: first.from,
      options: found.filter((one) => one.from === first.from).flatMap((one) => one.options)
    }
  }

  /** Project names from the symbol index, nearest first. */
  async function fromProject(context: CompletionContext, word: string): Promise<CompletionResult | null> {
    const path = options.path()
    if (!MEMBERS.test(path) || (PHP.test(path) && context.state.sliceDoc(context.pos - word.length - 1, context.pos - word.length) === '$')) {
      return null
    }
    if (!(await settle(context))) return null
    const mine = ++latestNames
    const found = await invokeAt<SymbolItem[]>(options.repo, 'complete_symbols', {
      prefix: word,
      path,
      limit: SYMBOL_LIMIT,
      changed: options.changed()
    }).catch(() => [] as SymbolItem[])
    if (mine !== latestNames || context.aborted || !found?.length) return null
    return {
      from: context.pos - word.length,
      validFor: found.length < SYMBOL_LIMIT ? /^[\w$]*$/ : undefined,
      options: found.map(
        (item, rank): Completion => ({
          label: item.name,
          type: item.kind,
          detail: folder(item.path),
          // The backend's order among names that match as well.
          boost: Math.max(1, 40 - rank),
          apply: (view, completion, from, to) => {
            view.dispatch({
              ...insertCompletionText(view.state, item.name, from, to),
              annotations: pickedCompletion.of(completion),
              userEvent: 'input.complete'
            })
            if (importable(path, item)) void addImport(view, path, item)
          }
        })
      )
    }
  }

  /** The `use` line a picked PHP class needs, added once the backend says where. */
  async function addImport(view: EditorView, path: string, item: SymbolItem, tries = 2): Promise<void> {
    const doc = view.state.doc
    const insert = await invokeAt<{ offset: number; text: string } | null>(options.repo, 'complete_import', {
      path,
      text: doc.toString(),
      classPath: item.path,
      name: item.name
    }).catch(() => null)
    if (!insert || !view.dom.isConnected) return
    // Typed into meanwhile: the offset is for a text that is gone.
    if (view.state.doc !== doc) {
      if (tries > 0) await addImport(view, path, item, tries - 1)
      return
    }
    view.dispatch({ changes: { from: insert.offset, insert: insert.text }, userEvent: 'input.complete' })
  }

  /** A word being typed: the language's, the file's words and the project's names, one list. */
  const words: CompletionSource = async (context) => {
    if (!allowed(context)) return null
    const before = context.state.sliceDoc(Math.max(0, context.pos - 200), context.pos)
    // After `->`, `::` and `.` only the receiver's members are offered.
    if (memberAccess(options.path(), before)) return null
    const word = /[\w$]*$/.exec(before)?.[0] ?? ''
    const plain = word.replace(/^\$/, '')
    if (!context.explicit && plain.length < 2) {
      return null
    }
    const [language, project, local] = await Promise.all([
      fromLanguage(context),
      plain.length ? fromProject(context, plain) : Promise.resolve(null),
      Promise.resolve(completeAnyWord(context))
    ])
    if (context.aborted) return null
    const from = context.pos - plain.length
    // A language list that starts elsewhere (after `<` in HTML) is its own.
    if (language && language.from !== from) return language
    const seen = new Set<string>()
    const merged: Completion[] = []
    for (const one of [...(project?.options ?? []), ...(language?.options ?? [])]) {
      if (seen.has(one.label)) continue
      seen.add(one.label)
      merged.push(one)
    }
    for (const one of local?.options ?? []) {
      if (seen.has(one.label) || one.label === plain) continue
      seen.add(one.label)
      merged.push({ ...one, boost: -10 })
    }
    if (!merged.length) return null
    return {
      from,
      options: merged,
      validFor: project && !project.validFor ? undefined : /^[\w$]*$/
    }
  }

  return [
    autocompletion({
      override: [members, words],
      defaultKeymap: false,
      activateOnTypingDelay: 30,
      closeOnBlur: true,
      icons: true,
      tooltipClass: () => 'gn-complete'
    }),
    completionKeys,
    keymap.of([
      { key: 'Ctrl-Space', run: startCompletion },
      { mac: 'Alt-`', run: startCompletion },
      ...closeBracketsKeymap
    ]),
    closeBrackets(),
    completionLook
  ]
}

/**
 * The keys the open list takes. Run ahead of every keymap, and the press
 * stops here when the list used it, so neither the editor's own Tab (indent)
 * and ↵ (new line) nor the window's Esc see it. With no list open they all
 * fall through untouched.
 */
const LIST_KEYS: Record<string, Command> = {
  Enter: acceptCompletion,
  Tab: acceptCompletion,
  Escape: closeCompletion,
  ArrowDown: moveCompletionSelection(true),
  ArrowUp: moveCompletionSelection(false),
  PageDown: moveCompletionSelection(true, 'page'),
  PageUp: moveCompletionSelection(false, 'page')
}

const completionKeys = Prec.highest(
  EditorView.domEventHandlers({
    keydown(event, view) {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false
      const command = LIST_KEYS[event.key]
      if (!command || completionStatus(view.state) !== 'active') return false
      if (!command(view)) return false
      event.preventDefault()
      event.stopPropagation()
      return true
    }
  })
)

/** Whether a list is open in the editor, for the window's own key handlers. */
export function listOpen(view: EditorView | null): boolean {
  return !!view && completionStatus(view.state) === 'active'
}

/**
 * The list in the app's colours, like the context menu and the usages
 * popup: raised, the code font at the code size, a letter per kind. Above
 * the editor and its change card (20), below dialogs (50); inside the
 * search window it is in the window's own layer, so over its preview.
 */
const completionLook = EditorView.theme({
  '.cm-tooltip.gn-complete': {
    zIndex: '40',
    background: 'var(--canvas)',
    color: 'var(--text)',
    border: '1px solid var(--line)',
    borderRadius: '8px',
    boxShadow: 'var(--shadow-pop)',
    overflow: 'hidden',
    padding: '3px'
  },
  '.cm-tooltip.gn-complete > ul': {
    fontFamily: 'var(--mono)',
    fontSize: 'var(--code-size, 12px)',
    maxHeight: 'calc(var(--code-row, 18px) * 12)',
    minWidth: '260px'
  },
  '.cm-tooltip.gn-complete > ul > li': {
    padding: '1px 8px 1px 4px',
    lineHeight: 'var(--code-row, 18px)',
    borderRadius: '4px'
  },
  '.cm-tooltip.gn-complete > ul > li[aria-selected]': {
    background: 'var(--active)',
    color: 'var(--text)'
  },
  '.cm-completionMatchedText': {
    textDecoration: 'none',
    color: 'var(--primary)',
    fontWeight: '600'
  },
  '.cm-completionDetail': {
    marginLeft: '10px',
    fontStyle: 'normal',
    color: 'var(--text-faint)'
  },
  '.cm-completionIcon': {
    width: '1.1em',
    paddingRight: '6px',
    opacity: '1',
    fontFamily: 'var(--font)',
    fontSize: 'calc(var(--code-size, 12px) - 2px)',
    fontWeight: '600',
    textAlign: 'center'
  },
  '.cm-completionIcon-method:after, .cm-completionIcon-function:after': { content: "'m'", color: 'var(--info)' },
  '.cm-completionIcon-function:after': { content: "'f'" },
  '.cm-completionIcon-property:after, .cm-completionIcon-variable:after': {
    content: "'p'",
    color: 'var(--success)'
  },
  '.cm-completionIcon-variable:after': { content: "'v'" },
  '.cm-completionIcon-constant:after': { content: "'c'", color: 'var(--warning)' },
  '.cm-completionIcon-case:after': { content: "'e'", color: 'var(--warning)' },
  '.cm-completionIcon-class:after': { content: "'C'", color: 'var(--primary)' },
  '.cm-completionIcon-interface:after': { content: "'I'", color: 'var(--primary)' },
  '.cm-completionIcon-trait:after': { content: "'T'", color: 'var(--primary)' },
  '.cm-completionIcon-enum:after': { content: "'E'", color: 'var(--primary)' },
  '.cm-completionIcon-type:after': { content: "'t'", color: 'var(--primary)' },
  '.cm-completionIcon-keyword:after': { content: "'k'", color: 'var(--text-dim)' },
  '.cm-completionIcon-namespace:after': { content: "'n'", color: 'var(--text-dim)' },
  '.cm-completionIcon-text:after': {
    content: "'w'",
    fontSize: 'inherit',
    verticalAlign: 'baseline',
    color: 'var(--text-faint)'
  },
  '.cm-tooltip.cm-completionInfo': {
    background: 'var(--canvas)',
    color: 'var(--text-dim)',
    border: '1px solid var(--line)',
    borderRadius: '6px',
    boxShadow: 'var(--shadow-pop)',
    fontFamily: 'var(--font)',
    fontSize: '12px',
    padding: '4px 8px',
    maxWidth: '360px'
  }
})

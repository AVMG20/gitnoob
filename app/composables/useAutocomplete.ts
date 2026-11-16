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
import { history } from '@codemirror/commands'
import { language, syntaxTree } from '@codemirror/language'
import { Annotation, Prec, Transaction, type EditorState, type Extension } from '@codemirror/state'
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
const BLADE = /\.blade\.php$/i
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
  if (BLADE.test(path)) return false
  return PHP.test(path) && PHP.test(item.path) && item.kind !== 'function' && item.kind !== 'constant'
}

/**
 * Whether a class name written at `from` is one a `use` line would serve.
 * Not a qualified name (`\App\Models\User` says where it is itself), and not
 * the name in a `namespace` or a file-level `use` line, which would import
 * the class it is already importing. A trait's `use` inside a class does
 * need one.
 */
export function wantsImport(state: EditorState, from: number): boolean {
  const lead = state.sliceDoc(state.doc.lineAt(from).from, from)
  if (/\\\s*$/.test(lead)) return false
  if (/^\s*namespace\b/.test(lead)) return false
  if (!/^\s*use\b/.test(lead)) return true
  type Found = ReturnType<ReturnType<typeof syntaxTree>['resolveInner']>
  for (let node: Found | null = syntaxTree(state).resolveInner(from, -1); node; node = node.parent) {
    if (node.type.name === 'DeclarationList' || node.type.name === 'EnumBody') return true
  }
  return false
}

/** Marks the `use` line's transaction as part of the undo step of the name it is for. */
const withName = Annotation.define<boolean>()

/**
 * What PHP itself has, since `@codemirror/lang-php` offers nothing: the
 * words of the language that are typed all day. Not its functions.
 */
export const PHP_KEYWORDS = [
  'abstract', 'array', 'as', 'bool', 'break', 'callable', 'case', 'catch', 'class', 'clone', 'const',
  'continue', 'declare', 'default', 'do', 'echo', 'else', 'elseif', 'empty', 'enum', 'extends', 'false',
  'final', 'finally', 'float', 'fn', 'for', 'foreach', 'function', 'global', 'if', 'implements',
  'include', 'include_once', 'instanceof', 'insteadof', 'int', 'interface', 'isset', 'iterable', 'list',
  'match', 'mixed', 'namespace', 'never', 'new', 'null', 'object', 'parent', 'print', 'private',
  'protected', 'public', 'readonly', 'require', 'require_once', 'return', 'self', 'static', 'string',
  'switch', 'throw', 'trait', 'true', 'try', 'unset', 'use', 'void', 'while', 'yield'
].map((label): Completion => ({ label, type: 'keyword' }))

/** Whether the code at `pos` is PHP, rather than the HTML around it in a template. */
function inPhp(state: EditorState, pos: number) {
  const top = state.facet(language)
  return !!top && top.name === 'php' && top.isActiveAt(state, pos)
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
    if (!MEMBERS.test(path)) return null
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
            const needs = importable(path, item) && wantsImport(view.state, from)
            const at = Date.now()
            view.dispatch({
              ...insertCompletionText(view.state, item.name, from, to),
              annotations: pickedCompletion.of(completion),
              userEvent: 'input.complete'
            })
            if (needs) void addImport(view, path, item, at)
          }
        })
      )
    }
  }

  /**
   * The `use` line a picked PHP class needs, added once the backend says
   * where. When nothing was typed since the name went in, the line joins the
   * name's undo step, so one ⌘Z takes both back; after more typing it is a
   * step of its own.
   */
  async function addImport(view: EditorView, path: string, item: SymbolItem, picked: number, tries = 2): Promise<void> {
    const doc = view.state.doc
    const first = tries === 2
    const insert = await invokeAt<{ offset: number; text: string } | null>(options.repo, 'complete_import', {
      path,
      text: doc.toString(),
      classPath: item.path,
      name: item.name
    }).catch(() => null)
    if (!insert || !view.dom.isConnected) return
    // Lent to another editor of the file, or locked, meanwhile: this one
    // only follows now, and an edit made here would be in nobody's file.
    if (!view.state.facet(EditorView.editable) || view.state.readOnly) return
    // Typed into meanwhile: the offset is for a text that is gone.
    if (view.state.doc !== doc) {
      if (tries > 0) await addImport(view, path, item, picked, tries - 1)
      return
    }
    if (insert.offset < 0 || insert.offset > doc.length) return
    view.dispatch({
      changes: { from: insert.offset, insert: insert.text },
      annotations: first ? [withName.of(true), Transaction.time.of(picked)] : [],
      userEvent: first ? undefined : 'input.complete'
    })
  }

  /** A word being typed: the language's, the file's words and the project's names, one list. */
  const words: CompletionSource = async (context) => {
    if (!allowed(context)) return null
    const before = context.state.sliceDoc(Math.max(0, context.pos - 200), context.pos)
    // After `->`, `::` and `.` only the receiver's members are offered.
    if (memberAccess(options.path(), before)) return null
    const path = options.path()
    // With its `$`s: in PHP and JavaScript they are part of the name, and
    // the file's own words come with them. A list that starts after the `$`
    // would put `$total` behind the one already typed.
    const word = /[\w$]*$/.exec(before)?.[0] ?? ''
    const plain = word.replace(/\$/g, '')
    if (!context.explicit && plain.length < 2) {
      return null
    }
    const from = context.pos - word.length
    const php = PHP.test(path) && inPhp(context.state, context.pos)
    // A PHP variable is only ever one of the file's own words.
    const variable = PHP.test(path) && word.includes('$')
    const [language, project, local] = await Promise.all([
      variable ? Promise.resolve(null) : fromLanguage(context),
      plain.length && !variable ? fromProject(context, word) : Promise.resolve(null),
      Promise.resolve(completeAnyWord(context))
    ])
    if (context.aborted) return null
    // A language list that starts elsewhere (after `<` in HTML) is its own.
    if (language && language.from !== from) return language
    const seen = new Set<string>()
    const merged: Completion[] = []
    // Two classes of one name in different folders are two rows.
    const rows = new Set<string>()
    for (const one of project?.options ?? []) {
      const row = `${one.label}\u0001${one.detail ?? ''}`
      if (rows.has(row)) continue
      rows.add(row)
      seen.add(one.label)
      merged.push(one)
    }
    for (const one of [...(language?.options ?? []), ...(php && !variable ? PHP_KEYWORDS : [])]) {
      if (seen.has(one.label)) continue
      seen.add(one.label)
      merged.push(one)
    }
    // The file's words, where they start at the same place as the rest.
    const words = local && local.from === from ? local.options : []
    for (const one of words) {
      if (seen.has(one.label) || one.label === word) continue
      seen.add(one.label)
      merged.push({ ...one, boost: -10 })
    }
    if (!merged.length) {
      // Only words that start elsewhere (a language that does not count `$`
      // as part of one): theirs is the list, from where they start.
      return local && local.options.length ? local : null
    }
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
    // Added to the editor's history rather than replacing it: the `use` line
    // of a picked class joins the step that put the name in.
    history({ joinToEvent: (tr) => tr.annotation(withName) === true }),
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
      // An input method's ↵ picks its own candidate, not ours.
      if (event.isComposing || event.keyCode === 229) return false
      const command = LIST_KEYS[event.key]
      if (!command || completionStatus(view.state) !== 'active') return false
      if (!command(view)) return false
      event.preventDefault()
      event.stopPropagation()
      return true
    }
  })
)

/**
 * Closes the list and drops what is still being asked. For a state on its
 * way to another editor or to the warm cache: the list was for this view's
 * caret and focus, and would come back open with the tab.
 */
export function closeList(view: EditorView | null): void {
  if (view) closeCompletion(view)
}

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

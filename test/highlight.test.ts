import { beforeAll, describe, expect, it } from 'vitest'
import {
  highlightLine,
  highlightWhole,
  labelFor,
  languageFor,
  ready
} from '../app/composables/useHighlight'

// Grammars are loaded rather than compiled in, so nothing is coloured until the
// highlighter is up.
beforeAll(() => ready())

/** Tags out, to check the text survived being coloured. */
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, '')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/&#39;/g, "'")

/** How many tokens got a colour of their own. */
const coloured = (html: string) => (html.match(/<span style="color:/g) ?? []).length

describe('languageFor', () => {
  it('reads an ordinary extension', () => {
    expect(languageFor('src/app.ts')).toBe('typescript')
    expect(languageFor('Makefile')).toBe('make')
  })

  it('colours a dotfile rather than treating its name as an extension', () => {
    expect(languageFor('.gitignore')).toBe('ini')
    expect(languageFor('.editorconfig')).toBe('ini')
    expect(languageFor('.prettierrc')).toBe('json')
  })

  it('takes a dotfile suffix as a variant, not as a type', () => {
    expect(languageFor('.env.local')).toBe('ini')
    expect(languageFor('.env.production')).toBe('ini')
  })

  it('still prefers a real extension on a dotfile that has one', () => {
    expect(languageFor('.eslintrc.json')).toBe('json')
    expect(languageFor('.eslintrc.yml')).toBe('yaml')
  })

  it('matches a named file whatever its case', () => {
    expect(languageFor('Dockerfile')).toBe('docker')
    expect(languageFor('deploy/DOCKERFILE')).toBe('docker')
    expect(languageFor('CMakeLists.txt')).toBe('cmake')
  })

  it('reads a file inside a directory by the file, not the path', () => {
    expect(languageFor('some.dir/app')).toBeNull()
    expect(languageFor('some.dir/app.rs')).toBe('rust')
  })

  it('gives back null for something it has no grammar for', () => {
    expect(languageFor('binary.bin')).toBeNull()
    expect(languageFor('LICENSE')).toBeNull()
  })
})

describe('labelFor', () => {
  it('names a file by what it is, not by the grammar used on it', () => {
    // The grammar is an implementation detail; the chip names the file.
    expect(labelFor('App.vue')).toBe('vue')
    expect(labelFor('main.rs')).toBe('rust')
    expect(labelFor('a.cpp')).toBe('c++')
  })

  it('names a dotfile after itself', () => {
    expect(labelFor('.gitignore')).toBe('.gitignore')
    expect(labelFor('.env.local')).toBe('.env')
  })

  it('has nothing to say about a file with no extension it knows', () => {
    expect(labelFor('LICENSE')).toBeNull()
  })
})

describe('highlightWhole', () => {
  const SFC = [
    '<template>',
    '  <p>{{ label }}</p>',
    '</template>',
    '',
    '<script setup lang="ts">',
    '// a comment',
    "const label = ref('x')",
    '</script>'
  ].join('\n')

  it('hands back one line of html for every line of source', () => {
    const lines = highlightWhole(SFC, 'vue')
    expect(lines).toHaveLength(SFC.split('\n').length)
    expect(text(lines.join('\n'))).toBe(SFC)
  })

  /**
   * The whole reason for the change. The old highlighter painted a `.vue` file
   * with the xml grammar, which left the script block as markup.
   */
  it('reads the script block of a single-file component as TypeScript', () => {
    const whole = highlightWhole(SFC, 'vue')
    // `const`, the name, `ref` and the string all get a colour of their own.
    expect(coloured(whole[6]!)).toBeGreaterThan(2)
    // And the template above it is coloured as markup, not as TypeScript.
    expect(coloured(whole[1]!)).toBeGreaterThan(0)
  })

  it('colours a comment inside the script block, which xml never could', () => {
    expect(coloured(highlightWhole(SFC, 'vue')[5]!)).toBeGreaterThan(0)
  })

  it('escapes the source rather than letting it become markup', () => {
    const source = '<img src=x onerror=alert(1)>'
    // The tag is split across spans, so it is the absence of real markup and
    // the text coming back whole that say the escaping held.
    const html = highlightWhole(source, 'html').join('')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;')
    expect(text(html)).toBe(source)
  })

  it('leaves text alone when there is no grammar for it', () => {
    const lines = highlightWhole('a < b\nc', null)
    expect(lines).toEqual(['a &lt; b', 'c'])
  })
})

/**
 * A review shows hunks and nothing else, so a pull request touching the script
 * body of a component carries no `<script>` tag at all — and the `vue` grammar,
 * shown a fragment with no block tag in it, reads the lot as template text.
 */
describe('a piece of a single-file component, with no block tag in it', () => {
  const script = ["const label = ref('x')", 'function go() {', '  return label.value', '}'].join('\n')
  const style = ['.pane {', '  color: red;', '  display: flex;', '}'].join('\n')
  const template = ['<div class="pane">', '  <p>{{ label }}</p>', '</div>'].join('\n')

  it('reads a script fragment as TypeScript rather than as markup', () => {
    expect(coloured(highlightWhole(script, 'vue').join(''))).toBeGreaterThan(3)
  })

  it('reads a style fragment as CSS', () => {
    expect(coloured(highlightWhole(style, 'vue').join(''))).toBeGreaterThan(3)
  })

  it('leaves a template fragment to the component grammar, which handles it', () => {
    const lines = highlightWhole(template, 'vue')
    expect(coloured(lines.join(''))).toBeGreaterThan(2)
    expect(text(lines.join('\n'))).toBe(template)
  })

  it('keeps the text exactly, whichever grammar it settled on', () => {
    for (const fragment of [script, style, template]) {
      expect(text(highlightWhole(fragment, 'vue').join('\n'))).toBe(fragment)
    }
  })

  it('leaves a whole component alone, since its tags already say what is what', () => {
    const whole = highlightWhole(SFC_WITH_STYLE, 'vue')
    expect(text(whole.join('\n'))).toBe(SFC_WITH_STYLE)
    // The style block is CSS and the script block is TypeScript, in one file.
    expect(coloured(whole[6]!)).toBeGreaterThan(1)
    expect(coloured(whole[10]!)).toBeGreaterThan(1)
  })
})

const SFC_WITH_STYLE = [
  '<template>',
  '  <p>{{ label }}</p>',
  '</template>',
  '',
  '<script setup lang="ts">',
  '// a comment',
  "const label = ref('x')",
  '</script>',
  '',
  '<style scoped>',
  '.pane { color: red; }',
  '</style>'
].join('\n')

describe('filenames that used to come out with no grammar at all', () => {
  it('reads a lock file that is really TOML', () => {
    expect(languageFor('Cargo.lock')).toBe('toml')
  })

  it('reads a Dockerfile whose suffix names an environment, not a type', () => {
    expect(languageFor('Dockerfile.prod')).toBe('docker')
    expect(languageFor('docker/Dockerfile.dev')).toBe('docker')
  })
})

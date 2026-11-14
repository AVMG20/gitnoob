import { describe, expect, it } from 'vitest'
import { detectIndent, languageIndent, resolveIndent } from '~/composables/indentation'

const PHP = `<?php

namespace App\\Console\\Commands;

class MakeThing extends Command
{
    /**
     * The name.
     */
    protected $signature = 'make:thing';

    public function handle(): int
    {
        $type = $this->argument('type');
        if ($type) {
            return 1;
        }
        return 0;
    }
}
`

const TS = `export function total(items: Item[]) {
  let sum = 0
  for (const item of items) {
    sum += item.price
  }
  return sum
}
`

const GO = 'package main\n\nfunc main() {\n\tif ok {\n\t\tfmt.Println("x")\n\t}\n}\n'

describe('reading indentation off a file', () => {
  it('reads four spaces from a PHP class, doc comments and all', () => {
    expect(detectIndent(PHP)).toEqual({ tabs: false, size: 4 })
  })

  it('reads two spaces from TypeScript', () => {
    expect(detectIndent(TS)).toEqual({ tabs: false, size: 2 })
  })

  it('reads tabs from Go', () => {
    expect(detectIndent(GO)?.tabs).toBe(true)
  })

  it('goes with the majority in a mixed file', () => {
    const mixed = PHP + '\tstray();\n'
    expect(detectIndent(mixed)).toEqual({ tabs: false, size: 4 })
    expect(detectIndent('a\n\tb\n\tc\n    d\n')?.tabs).toBe(true)
  })

  it('says nothing for an empty or flat file', () => {
    expect(detectIndent('')).toBeNull()
    expect(detectIndent('one\ntwo\n')).toBeNull()
  })

  it('knows what each language usually uses', () => {
    expect(languageIndent('a.php')).toEqual({ tabs: false, size: 4 })
    expect(languageIndent('src/a.vue')).toEqual({ tabs: false, size: 2 })
    expect(languageIndent('main.go')?.tabs).toBe(true)
    expect(languageIndent('Makefile')?.tabs).toBe(true)
    expect(languageIndent('notes.txt')).toBeNull()
  })
})

describe('deciding the indentation', () => {
  it('lets an override beat .editorconfig, which beats the file', () => {
    const editorconfig = { style: 'space', size: 2 }
    expect(resolveIndent({ path: 'a.php', text: PHP, editorconfig }).size).toBe(2)
    expect(resolveIndent({ path: 'a.php', text: PHP, editorconfig, override: { tabs: true, size: 0 } }).tabs).toBe(true)
    expect(resolveIndent({ path: 'a.php', text: PHP }).from).toBe('file')
  })

  it('falls back to Settings when they say Spaces or Tabs, then to the language', () => {
    const flat = 'one\n'
    expect(resolveIndent({ path: 'a.ts', text: flat, settings: { style: 'tabs' } }).tabs).toBe(true)
    expect(resolveIndent({ path: 'a.ts', text: flat, settings: { style: 'detect', size: 8 } })).toMatchObject({
      tabs: false,
      size: 2,
      from: 'language'
    })
    expect(resolveIndent({ path: 'a.txt', text: flat, settings: { style: 'detect', size: 8 } }).size).toBe(8)
  })

  it('draws a tab as wide as .editorconfig or Settings say', () => {
    expect(resolveIndent({ path: 'main.go', text: GO, settings: { tabWidth: 8 } }).tabWidth).toBe(8)
    expect(resolveIndent({ path: 'main.go', text: GO, editorconfig: { style: 'tab', tab_width: 2 } }).tabWidth).toBe(2)
  })
})

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { contrast } from '../scripts/theme/color.mjs'

/**
 * The generated files, checked against what the generator would write now.
 *
 * A generated file that can be hand-edited is a generated file that will be,
 * and the edit is lost the next time somebody runs the script. This is the only
 * thing that keeps the two honest.
 */
describe('the theme files', () => {
  const files = ['app/assets/css/themes.css', 'app/composables/themeList.ts']

  it('are what the palette says they should be', () => {
    const before = files.map((path) => readFileSync(path, 'utf8'))
    execFileSync('node', ['scripts/theme/generate.mjs'])
    const after = files.map((path) => readFileSync(path, 'utf8'))
    // Running the generator is how the question is asked, so put back what was
    // there before answering it: a failing check should not also leave two
    // rewritten files behind for somebody to notice later.
    files.forEach((path, at) => writeFileSync(path, before[at]!))
    // The generator writes bare newlines. A Windows checkout of these files has
    // CRLF in it, and comparing the two would fail on every machine that is not
    // the one they were generated on, which says nothing about the palette.
    const body = (text: string) => text.replace(/\r\n/g, '\n')
    files.forEach((path, at) => {
      expect(body(after[at]!), `${path} is out of date — run \`npm run theme\``).toBe(
        body(before[at]!)
      )
    })
  })

  it('names no colour outside them', () => {
    // Every hardcoded rgb in a component is a colour one theme cannot change.
    const found = execFileSync('sh', [
      '-c',
      "grep -rno 'rgba\\?([0-9][^)]*)' app/components/*.vue app/assets/css/main.css || true"
    ])
      .toString()
      .trim()
    expect(found).toBe('')
  })

  /**
   * What every theme is worth, read off the file that ships.
   *
   * The point of stating the ladder as contrast rather than as lightness is
   * that it can be checked, and a palette nobody checks drifts back to
   * "looks about right on my screen". Each of these is a thing somebody has to
   * be able to read: dimmed text on a panel, a label on a filled button, a
   * green that says a line was added.
   */
  it('is legible in every theme, at every contrast setting', () => {
    const css = readFileSync('app/assets/css/themes.css', 'utf8')
    const blocks = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((one) => ({
      selector: one[1]!.trim(),
      vars: Object.fromEntries(
        [...one[2]!.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((v) => [v[1]!, v[2]!.trim()])
      )
    }))

    const base = blocks.filter((one) => !one.selector.includes('data-contrast'))
    const failures: string[] = []

    for (const theme of base) {
      for (const level of ['normal', 'cosy', 'high']) {
        const over =
          level === 'normal'
            ? {}
            : (blocks.find((one) => one.selector === `${theme.selector}[data-contrast='${level}']`)
                ?.vars ?? {})
        const v = { ...theme.vars, ...over }
        const at = (name: string) => v[name] ?? ''
        const where = `${theme.selector} ${level}`

        const checks: [string, number, number][] = [
          ['label on a primary button', contrast(at('--primary-fg'), at('--primary')), 4.5],
          ['label on a danger button', contrast(at('--danger-fg'), at('--danger')), 4.5],
          ['dim text on the page', contrast(at('--fg-muted'), at('--bg')), 5],
          ['faint text on a panel', contrast(at('--fg-subtle'), at('--surface')), level === 'cosy' ? 3 : 4],
          ['an added line', contrast(at('--success'), at('--bg')), 3],
          ['a deleted line', contrast(at('--danger'), at('--bg')), 3],
          ['a warning', contrast(at('--warning'), at('--bg')), 3],
          ['the first graph lane', contrast(at('--lane-1'), at('--bg')), 3]
        ]
        for (const [what, got, want] of checks) {
          if (got < want) failures.push(`${where}: ${what} is ${got}, wanted ${want}`)
        }
      }
    }

    expect(failures).toEqual([])
  })
})

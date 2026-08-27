/**
 * Writes `app/assets/css/themes.css` from the palette.
 *
 * Nothing in that file is meant to be edited by hand: change a theme here and
 * run `npm run theme`. The check in the test suite fails when the two have
 * drifted, which is the only way a generated file stays generated.
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { contrast, oklchToHex, parseHex, round, toHex } from './color.mjs'
import { CONTRAST, LADDERS, LANE_HUES, THEMES } from './palette.mjs'

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
const out = path.join(root, 'app', 'assets', 'css', 'themes.css')
const list = path.join(root, 'app', 'composables', 'themeList.ts')

/** One colour laid over another at `alpha`, as the screen would show it. */
function over(top, bottom, alpha) {
  const a = parseHex(top)
  const b = parseHex(bottom)
  return toHex(a.map((one, at) => one * alpha + b[at] * (1 - alpha)))
}

/** A rung of the ladder: the page, moved in lightness, keeping its own hue. */
function rung(bg, by, chroma = 1) {
  return oklchToHex({ l: clamp(bg.l + by), c: round(bg.c * chroma, 4), h: bg.h })
}

const clamp = (value) => Math.min(1, Math.max(0, round(value, 4)))

/**
 * The lightness at which a colour is worth `target` against the page.
 *
 * Found rather than stated: it is the ratio that has to hold, and what
 * lightness delivers it depends on how dark the page is. A light theme searches
 * downwards, a dark one up, and both stop at the first step that clears the
 * bar.
 */
function atRatio(bg, target, hue, chroma, light) {
  let best = light ? '#000000' : '#ffffff'
  for (let step = 0; step <= 1000; step++) {
    const l = light ? 1 - step / 1000 : step / 1000
    const hex = oklchToHex({ l: round(l, 4), c: round(chroma, 4), h: hue })
    best = hex
    if (contrast(hex, bg) >= target) break
  }
  return best
}

/** Text that has to be read on the page rather than filled behind. */
function soft(colour, family) {
  return family === 'light'
    ? oklchToHex({ ...colour, l: clamp(colour.l - 0.09) })
    : oklchToHex({ l: clamp(colour.l + 0.09), c: round(colour.c * 0.85, 4), h: colour.h })
}

/** What can be read on top of a filled button of this colour. */
function onTop(colour) {
  return colour.l > 0.62
    ? oklchToHex({ l: 0.16, c: Math.min(colour.c, 0.04), h: colour.h })
    : '#ffffff'
}

/**
 * A meaning colour moved until it can do both of its jobs.
 *
 * The same token is a word on the page and a fill under one, and the two pull
 * in opposite directions: light enough to read on a dark page is too light to
 * put white on. So it is nudged — hue and chroma kept, lightness given up —
 * until it clears 3:1 against the page and its own label clears 4.5:1 against
 * it. The light themes are where this bites; a blue picked to look right on
 * white was carrying white text at 4:1.
 */
function readable(colour, bg) {
  let fitted = { ...colour }
  for (let step = 0; step < 60 && contrast(oklchToHex(fitted), bg) < 3; step++) {
    fitted.l = round(fitted.l + (contrast('#ffffff', bg) > 2 ? 0.01 : -0.01), 4)
  }
  const label = onTop(fitted)
  const away = label === '#ffffff' ? -0.01 : 0.01
  for (let step = 0; step < 60 && contrast(label, oklchToHex(fitted)) < 4.5; step++) {
    fitted.l = round(Math.min(0.97, Math.max(0.2, fitted.l + away)), 4)
  }
  return fitted
}

/** The ten graph lanes, in this theme's light. */
function lanes(theme, ladder) {
  const hues = [theme.primary.h, ...LANE_HUES.slice(1)]
  return hues.map((hue, at) => {
    // A lane that lands on the accent's hue would read as the trunk; the ones
    // that follow are nudged clear of it.
    const apart =
      at > 0 && Math.abs(((hue - theme.primary.h + 540) % 360) - 180) > 165 ? hue + 24 : hue
    return oklchToHex({ l: ladder.lane.l, c: ladder.lane.c, h: (apart + 360) % 360 })
  })
}

/** Every token a theme carries, at one contrast setting. */
function tokens(theme, level) {
  const ladder = LADDERS[theme.family]
  const shift = CONTRAST[level]
  const light = theme.family === 'light'

  const bg = oklchToHex(theme.bg)
  const surface = rung(theme.bg, ladder.surface)
  const raised = rung(theme.bg, ladder.raised)
  // Each meaning colour, moved to where it can be both read and written on.
  const seeds = {
    primary: readable(theme.primary, bg),
    danger: readable(theme.danger, bg),
    success: readable(theme.success, bg),
    warning: readable(theme.warning, bg),
    info: readable(theme.info, bg)
  }
  const primary = oklchToHex(seeds.primary)
  const danger = oklchToHex(seeds.danger)
  const success = oklchToHex(seeds.success)
  const warning = oklchToHex(seeds.warning)
  const info = oklchToHex(seeds.info)

  // Text and lines are placed by what they are worth against the page rather
  // than by where they sit on the ladder: the same lightness reads differently
  // on white and on black, and legibility is the thing being asked for.
  const chroma = Math.min(theme.bg.c * 1.6, 0.02)
  const fgAt = (target) => atRatio(bg, target, theme.bg.h, chroma, light)
  const lineAt = (target, tint) => atRatio(bg, target, theme.bg.h, theme.bg.c * tint, light)

  const map = {
    '--bg': bg,
    '--surface': surface,
    '--raised': raised,
    '--deep': rung(theme.bg, ladder.deep),
    '--hover': rung(theme.bg, ladder.hover),
    '--active': over(primary, bg, ladder.active),

    '--fg': fgAt(shift.fg),
    '--fg-muted': fgAt(shift.muted),
    '--fg-subtle': fgAt(shift.subtle),

    '--border': lineAt(shift.border, 1.6),
    '--border-soft': lineAt(shift.borderSoft, 1.3),
    '--ring': primary,

    '--primary': primary,
    '--primary-fg': onTop(seeds.primary),
    '--primary-hover': oklchToHex({
      ...seeds.primary,
      l: clamp(seeds.primary.l + (light ? -0.06 : 0.06))
    }),
    '--primary-soft': soft(seeds.primary, theme.family),
    '--primary-bg': over(primary, bg, ladder.tint),
    '--primary-line': over(primary, bg, ladder.tintLine),

    '--danger': danger,
    '--danger-fg': onTop(seeds.danger),
    '--danger-hover': oklchToHex({
      ...seeds.danger,
      l: clamp(seeds.danger.l + (light ? -0.06 : 0.06))
    }),
    '--danger-soft': soft(seeds.danger, theme.family),
    '--danger-bg': over(danger, bg, ladder.tint),
    '--danger-line': over(danger, bg, ladder.tintLine),

    '--success': success,
    '--success-soft': soft(seeds.success, theme.family),
    '--success-bg': over(success, bg, ladder.tint),
    '--success-line': over(success, bg, ladder.tintLine),

    '--warning': warning,
    '--warning-soft': soft(seeds.warning, theme.family),
    '--warning-bg': over(warning, bg, ladder.tint),
    '--warning-line': over(warning, bg, ladder.tintLine),

    '--info': info,
    '--info-soft': soft(seeds.info, theme.family),
    '--info-bg': over(info, bg, ladder.tint),

    // The diff colours are the intent colours, said again under names that mean
    // what they are for: what a theme wants for an added line is not always
    // what it wants for a success message.
    '--diff-add-bg': over(success, bg, ladder.tint),
    '--diff-add-line': over(success, bg, ladder.tintLine),
    '--diff-del-bg': over(danger, bg, ladder.tint),
    '--diff-del-line': over(danger, bg, ladder.tintLine),
    '--diff-ours': primary,
    '--diff-theirs': info,

    '--scrollbar': rung(theme.bg, ladder.scrollbar, 1.2),
    '--scrollbar-hover': rung(theme.bg, ladder.scrollbarHover, 1.2),
    '--overlay': alpha(rung(theme.bg, light ? -0.75 : -0.12), ladder.overlay),
    '--shadow': alpha('#000000', ladder.shadow),
    '--shadow-strong': alpha('#000000', ladder.shadowStrong)
  }

  lanes(theme, ladder).forEach((hex, at) => {
    map[`--lane-${at + 1}`] = hex
  })

  return { ...map, ...(theme.overrides ?? {}) }
}

/** `#rrggbb` as an `rgba()`, for the two places a colour has to be see-through. */
function alpha(hex, value) {
  const [r, g, b] = parseHex(hex).map((one) => Math.round(one * 255))
  return `rgba(${r}, ${g}, ${b}, ${value})`
}

/**
 * The old names, kept pointing at the new ones.
 *
 * Three hundred-odd rules across the app still say `--text-dim`, and renaming
 * them in the same change that moves every colour would mean auditing both at
 * once. They are two jobs, and this is the seam between them.
 */
const ALIASES = {
  '--bg-panel': '--surface',
  '--bg-raised': '--raised',
  '--bg-hover': '--hover',
  '--bg-active': '--active',
  '--bg-deep': '--deep',
  '--line': '--border',
  '--line-soft': '--border-soft',
  '--text': '--fg',
  '--text-dim': '--fg-muted',
  '--text-faint': '--fg-subtle',
  '--accent': '--primary',
  '--on-accent': '--primary-fg',
  '--accent-hover': '--primary-hover',
  '--accent-soft': '--primary-soft',
  '--red': '--danger',
  '--on-danger': '--danger-fg',
  '--red-soft': '--danger-soft',
  '--green': '--success',
  '--green-soft': '--success-soft',
  '--amber': '--warning',
  '--amber-soft': '--warning-soft',
  '--purple': '--info',
  '--purple-soft': '--info-soft'
}

const lines = (map, indent = '  ') =>
  Object.entries(map)
    .map(([name, value]) => `${indent}${name}: ${value};`)
    .join('\n')

function block(selector, map) {
  return `${selector} {\n${lines(map)}\n}`
}

function build() {
  const out = []
  out.push(`/*
 * Generated by \`npm run theme\` from scripts/theme/palette.mjs. Do not edit.
 *
 * Every theme states a ground and five meaning colours in OKLCH; the ladder its
 * family carries works out the rest. Shipped as hex because an unsupported
 * colour function is ignored rather than approximated, and the oldest webview
 * this app runs in is whatever the user's distribution shipped.
 */`)

  for (const theme of THEMES) {
    const selector = theme.name === 'slate' ? ':root' : `[data-theme='${theme.name}']`
    out.push(`\n/* ${theme.label} — ${theme.note} */`)
    out.push(block(selector, { ...tokens(theme, 'normal'), ...aliasBlock(theme) }))

    for (const level of ['cosy', 'high']) {
      const all = tokens(theme, level)
      const only = Object.fromEntries(
        ['--fg-muted', '--fg-subtle', '--border', '--border-soft'].map((key) => [key, all[key]])
      )
      out.push(block(`${selector}[data-contrast='${level}']`, only))
    }
  }

  return `${out.join('\n')}\n`
}

/** Aliases are written once per theme, since a theme may override one. */
function aliasBlock(theme) {
  const map = {}
  for (const [old, current] of Object.entries(ALIASES)) map[old] = `var(${current})`
  return map
}

/**
 * The list the settings page shows, written from the same palette.
 *
 * The swatches used to be three hexes typed next to each theme's name, and one
 * of them had drifted a whole hue away from the theme it claimed to show. They
 * are read off the tokens now, so a card cannot lie about what it opens.
 */
function themeList() {
  // How dark the page is, rather than which ladder it uses: a reader picking a
  // theme is asking "how dark is this?", and Obsidian is far darker than Slate
  // though both are built the same way.
  const kindOf = (theme) =>
    theme.bg.l > 0.5 ? 'Light' : theme.bg.l < 0.16 ? 'Dark' : 'Semi-dark'
  const rows = THEMES.map((theme) => {
    const t = tokens(theme, 'normal')
    return `  {
    id: '${theme.name}',
    name: '${theme.label}',
    kind: '${kindOf(theme)}',
    note: '${theme.note.replace(/'/g, "\\'")}',
    swatch: ['${t['--bg']}', '${t['--primary']}', '${t['--fg']}']
  }`
  })

  return `/**
 * Generated by \`npm run theme\` from scripts/theme/palette.mjs. Do not edit.
 *
 * The themes as the settings page lists them: what each one is called, which
 * group it belongs under, and the three colours its card is painted with.
 */

export type ThemeId =
${THEMES.map((theme) => `  | '${theme.name}'`).join('\n')}

export interface Theme {
  id: ThemeId
  name: string
  /** The group it belongs to, shown over the swatches. */
  kind: 'Light' | 'Semi-dark' | 'Dark'
  /** One line on what it is for. */
  note: string
  /** Background, accent, text — the three colours a card is painted with. */
  swatch: [string, string, string]
}

export const THEMES: Theme[] = [
${rows.join(',\n')}
]
`
}

const css = build()
fs.writeFileSync(out, css)
fs.writeFileSync(list, themeList())

// A word about where each theme landed, since contrast is the point of this.
const check = process.argv.includes('--report')
if (check) {
  const pad = (text, width) => String(text).padEnd(width)
  console.log(pad('theme', 11) + ['fg', 'muted', 'subtle', 'border', 'primary'].map((one) => one.padStart(9)).join(''))
  for (const theme of THEMES) {
    const t = tokens(theme, 'normal')
    const row = ['--fg', '--fg-muted', '--fg-subtle', '--border', '--primary']
      .map((key) => String(contrast(t[key], t['--bg'])).padStart(9))
      .join('')
    console.log(pad(theme.name, 11) + row)
  }
}

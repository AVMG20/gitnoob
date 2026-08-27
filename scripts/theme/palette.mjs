/**
 * What every theme is made of.
 *
 * A theme states its ground and its five meaning colours; everything else on
 * screen is worked out from them by the ladder its family carries. That is the
 * whole point of writing them in OKLCH: `l` is perceived lightness, so "one
 * step up out of the page" is the same step whatever hue the theme is in, and
 * seventeen themes can share a structure without sharing a look.
 *
 * The ladders were read back out of the themes as they were hand-written, so
 * the shape of the app does not move. Two rungs are deliberately different:
 * the faintest text was at 2.4–3.3:1 against the page and is now above 4:1,
 * and the lines that separate one surface from another were at 1.3:1, which is
 * a border nobody can see.
 */

/**
 * How far each surface sits from the page, in lightness.
 *
 * Negative goes down into the page — `deep` is the well a scrolling list sits
 * in — and positive comes up out of it.
 */
const LADDERS = {
  dark: {
    surface: 0.026,
    raised: 0.055,
    hover: 0.083,
    deep: -0.02,
    /** The tint of the theme's own colour on a selected row. */
    active: 0.2,
    /** Diff tints and the fills behind a warning or a danger note. */
    tint: 0.12,
    tintLine: 0.34,
    lane: { l: 0.7, c: 0.155 },
    scrollbar: 0.15,
    scrollbarHover: 0.25,
    shadow: 0.35,
    shadowStrong: 0.55,
    overlay: 0.62
  },
  /**
   * The black themes: the page is pure black and cannot be stepped down from,
   * so every rung is measured from the panel above it instead and the steps
   * are wider — on black, a difference of 0.02 in lightness is nothing.
   */
  black: {
    surface: 0.15,
    raised: 0.195,
    hover: 0.235,
    deep: 0,
    active: 0.3,
    tint: 0.14,
    tintLine: 0.36,
    lane: { l: 0.72, c: 0.16 },
    scrollbar: 0.2,
    scrollbarHover: 0.3,
    shadow: 0.6,
    shadowStrong: 0.75,
    overlay: 0.7
  },
  light: {
    surface: -0.022,
    raised: -0.05,
    hover: -0.07,
    deep: -0.045,
    active: -0.08,
    tint: 0.14,
    tintLine: 0.38,
    lane: { l: 0.55, c: 0.16 },
    scrollbar: -0.14,
    scrollbarHover: -0.24,
    shadow: 0.14,
    shadowStrong: 0.22,
    overlay: 0.45
  }
}

/**
 * The five rungs that are stated as contrast rather than as lightness.
 *
 * Every other rung is a step out of the page and can be said in lightness; text
 * cannot, because "readable" is a ratio against what it sits on and the same
 * lightness is worth different amounts on white and on black. Naming the ratio
 * is what makes a theme's legibility a decision instead of an accident — and it
 * is what lets one setting raise it across all eighteen of them.
 *
 * 4.5 is what WCAG asks of body text, and the faintest text in the app was at
 * 2.4 to 3.3 before this.
 *
 * The lines are deliberately below what the same standard asks of a control's
 * edge. Most of the lines in this window group things rather than bound them —
 * the cards down the side of a review, the rule under a heading — and a window
 * of boxes all drawn at 3:1 reads as a grid, not as a page. High contrast is
 * where they step up for the people who need them to.
 */
const CONTRAST = {
  cosy: { fg: 12, muted: 5.4, subtle: 3.5, border: 1.35, borderSoft: 1.15 },
  normal: { fg: 13, muted: 6.5, subtle: 4.5, border: 1.62, borderSoft: 1.28 },
  high: { fg: 15.5, muted: 8.5, subtle: 6.2, border: 2.6, borderSoft: 1.75 }
}

/**
 * The ten lanes of the graph, by hue.
 *
 * Told apart at a glance is the only thing being asked of them, so the hues are
 * fixed and it is the lightness and chroma that follow the theme — a lane on
 * white has to be darker than the same lane on black to read as the same line.
 * The first is the trunk's, and it takes the theme's own accent hue.
 */
const LANE_HUES = [254.6, 73, 155.7, 13.7, 300, 205, 350, 130, 40, 275]

/** Themes, in the order the settings list shows them. */
const THEMES = [
  {
    name: 'slate',
    label: 'Slate',
    family: 'dark',
    note: 'The default: a cool grey window with a blue accent.',
    bg: { l: 0.2071, c: 0.0117, h: 254.1 },
    primary: { l: 0.687, c: 0.1575, h: 254.6 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'fjord',
    label: 'Fjord',
    family: 'dark',
    note: 'Slate with more blue in the ground.',
    bg: { l: 0.2184, c: 0.0223, h: 245.8 },
    primary: { l: 0.7069, c: 0.1201, h: 232.4 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'pine',
    label: 'Pine',
    family: 'dark',
    note: 'A green-grey window; the accent is a deep teal.',
    bg: { l: 0.1802, c: 0.0175, h: 168.9 },
    primary: { l: 0.7207, c: 0.1145, h: 178.5 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'dusk',
    label: 'Dusk',
    family: 'dark',
    note: 'Warm grey, with a violet accent.',
    bg: { l: 0.2197, c: 0.0146, h: 308.6 },
    primary: { l: 0.6957, c: 0.1509, h: 296.5 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'plum',
    label: 'Plum',
    family: 'dark',
    note: 'A purple-tinted window and a pink accent.',
    bg: { l: 0.2054, c: 0.0227, h: 327.3 },
    primary: { l: 0.7223, c: 0.1512, h: 350.1 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'moss',
    label: 'Moss',
    family: 'dark',
    note: 'An olive window with a lime accent.',
    bg: { l: 0.2127, c: 0.0208, h: 128.4 },
    primary: { l: 0.7935, c: 0.1584, h: 129.4 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'ink',
    label: 'Ink',
    family: 'dark',
    note: 'Nearly black, with a colder blue.',
    bg: { l: 0.1676, c: 0.0136, h: 254.6 },
    primary: { l: 0.6759, c: 0.1461, h: 249.6 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'obsidian',
    label: 'Obsidian',
    family: 'dark',
    note: 'Neutral charcoal, no colour in the ground at all.',
    bg: { l: 0.1395, c: 0.0026, h: 285 },
    primary: { l: 0.6663, c: 0.1465, h: 255.8 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'mono',
    label: 'Mono',
    family: 'dark',
    note: 'Grey, and one colour: everything that is not meaning is grey.',
    bg: { l: 0.1448, c: 0, h: 0 },
    primary: { l: 0.9219, c: 0.0033, h: 286.4 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7323, c: 0.1333, h: 155.7 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 },
    /** Grey is the accent, so a filled button needs the page back on top. */
    overrides: { '--primary-fg': 'var(--bg)' }
  },
  {
    name: 'void',
    label: 'Void',
    family: 'black',
    note: 'Pure black, with a cyan accent.',
    bg: { l: 0, c: 0, h: 254 },
    primary: { l: 0.7239, c: 0.1105, h: 205.6 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7752, c: 0.1626, h: 155.9 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'jade',
    label: 'Jade',
    family: 'black',
    note: 'Black, lit green — the additions move yellow to stay apart from it.',
    bg: { l: 0, c: 0, h: 150 },
    primary: { l: 0.8005, c: 0.1968, h: 148.6 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.8543, c: 0.176, h: 118 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'crimson',
    label: 'Crimson',
    family: 'black',
    note: 'Black, lit red — a deletion keeps a redder red to stay apart.',
    bg: { l: 0, c: 0, h: 20 },
    primary: { l: 0.6631, c: 0.2124, h: 22.9 },
    danger: { l: 0.5866, c: 0.2011, h: 8 },
    success: { l: 0.7752, c: 0.1626, h: 155.9 },
    warning: { l: 0.7831, c: 0.1453, h: 73 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'ember',
    label: 'Ember',
    family: 'black',
    note: 'Black, lit amber — a warning moves yellower to stay apart.',
    bg: { l: 0, c: 0, h: 60 },
    primary: { l: 0.7444, c: 0.1652, h: 55.1 },
    danger: { l: 0.6409, c: 0.171, h: 13.7 },
    success: { l: 0.7752, c: 0.1626, h: 155.9 },
    warning: { l: 0.8828, c: 0.16, h: 90 },
    info: { l: 0.6782, c: 0.171, h: 300 }
  },
  {
    name: 'paper',
    label: 'Paper',
    family: 'light',
    note: 'Plain white, blue accent.',
    bg: { l: 1, c: 0, h: 0 },
    primary: { l: 0.5985, c: 0.1663, h: 255.5 },
    danger: { l: 0.5976, c: 0.1785, h: 15.1 },
    success: { l: 0.6206, c: 0.1415, h: 152.7 },
    warning: { l: 0.6653, c: 0.1358, h: 68.7 },
    info: { l: 0.6004, c: 0.2118, h: 293.3 }
  },
  {
    name: 'mist',
    label: 'Mist',
    family: 'light',
    note: 'A cool off-white, easier on the eyes than plain white.',
    bg: { l: 0.9578, c: 0.0058, h: 264.5 },
    primary: { l: 0.5867, c: 0.1664, h: 259.1 },
    danger: { l: 0.618, c: 0.1802, h: 15.3 },
    success: { l: 0.6225, c: 0.1335, h: 155.5 },
    warning: { l: 0.6746, c: 0.1353, h: 72.2 },
    info: { l: 0.6061, c: 0.2093, h: 294.1 }
  },
  {
    name: 'sand',
    label: 'Sand',
    family: 'light',
    note: 'Warm paper, for people who find white screens cold.',
    bg: { l: 0.9771, c: 0.0119, h: 91.2 },
    primary: { l: 0.5793, c: 0.1372, h: 250 },
    danger: { l: 0.5976, c: 0.1785, h: 15.1 },
    success: { l: 0.6206, c: 0.1415, h: 152.7 },
    warning: { l: 0.6653, c: 0.1358, h: 68.7 },
    info: { l: 0.6004, c: 0.2118, h: 293.3 }
  },
  {
    name: 'frost',
    label: 'Frost',
    family: 'light',
    note: 'Pale blue paper with a teal accent.',
    bg: { l: 0.9831, c: 0.0102, h: 214.4 },
    primary: { l: 0.5648, c: 0.1005, h: 220.7 },
    danger: { l: 0.5976, c: 0.1785, h: 15.1 },
    success: { l: 0.6206, c: 0.1415, h: 152.7 },
    warning: { l: 0.6653, c: 0.1358, h: 68.7 },
    info: { l: 0.6004, c: 0.2118, h: 293.3 }
  },
  {
    name: 'lilac',
    label: 'Lilac',
    family: 'light',
    note: 'Pale violet paper, purple accent.',
    bg: { l: 0.9851, c: 0.0136, h: 306.4 },
    primary: { l: 0.5137, c: 0.2059, h: 296.4 },
    danger: { l: 0.5976, c: 0.1785, h: 15.1 },
    success: { l: 0.6206, c: 0.1415, h: 152.7 },
    warning: { l: 0.6653, c: 0.1358, h: 68.7 },
    info: { l: 0.6004, c: 0.2118, h: 293.3 }
  }
]

export { THEMES, LADDERS, CONTRAST, LANE_HUES }

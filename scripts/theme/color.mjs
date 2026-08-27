/**
 * sRGB and OKLCH, both ways.
 *
 * OKLCH is how the themes are written: `L` is perceived lightness, so a ladder
 * of surfaces stated in it steps by the same amount whatever hue it is in —
 * which is the whole reason the themes can share a structure and still look
 * like themselves. Hex is what ships, because an unsupported colour function is
 * ignored rather than approximated, and the oldest webview this app runs in is
 * whatever the user's Linux distribution shipped.
 *
 * The conversion is Björn Ottosson's, unchanged.
 */

const clamp = (value, low = 0, high = 1) => Math.min(high, Math.max(low, value))

const toLinear = (value) =>
  value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4

const toGamma = (value) =>
  value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055

/** `#rgb` or `#rrggbb` to three 0–1 channels. */
export function parseHex(hex) {
  let text = hex.trim().replace('#', '')
  if (text.length === 3) text = [...text].map((one) => one + one).join('')
  return [0, 2, 4].map((at) => parseInt(text.slice(at, at + 2), 16) / 255)
}

export function toHex([r, g, b]) {
  const byte = (value) =>
    Math.round(clamp(value) * 255)
      .toString(16)
      .padStart(2, '0')
  return `#${byte(r)}${byte(g)}${byte(b)}`
}

/** sRGB (0–1) to OKLab. */
function srgbToOklab([r, g, b]) {
  const lr = toLinear(r)
  const lg = toLinear(g)
  const lb = toLinear(b)
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb)
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb)
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb)
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
  ]
}

/** OKLab to sRGB (0–1), which may be outside the cube. */
function oklabToSrgb([L, a, b]) {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3
  return [
    toGamma(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    toGamma(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    toGamma(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s)
  ]
}

/** `{ l, c, h }` — lightness 0–1, chroma, hue in degrees. */
export function hexToOklch(hex) {
  const [L, a, b] = srgbToOklab(parseHex(hex))
  const c = Math.sqrt(a * a + b * b)
  let h = (Math.atan2(b, a) * 180) / Math.PI
  if (h < 0) h += 360
  return { l: round(L, 4), c: round(c, 4), h: round(c < 0.0002 ? 0 : h, 1) }
}

/**
 * OKLCH to the nearest hex the screen can actually show.
 *
 * A lightness and a chroma can name a colour sRGB has no room for, and the
 * channels then land outside 0–1. Chroma is walked down until it fits, which
 * keeps the lightness — the thing the ladder is built on — and gives up only
 * the saturation that was never going to be displayed.
 */
export function oklchToHex({ l, c, h }) {
  const radians = (h * Math.PI) / 180
  for (let chroma = c; chroma >= 0; chroma -= 0.002) {
    const rgb = oklabToSrgb([l, chroma * Math.cos(radians), chroma * Math.sin(radians)])
    if (rgb.every((one) => one >= -0.001 && one <= 1.001)) return toHex(rgb)
  }
  return toHex(oklabToSrgb([l, 0, 0]))
}

/** WCAG relative luminance, for saying what a pair of colours is worth. */
export function luminance(hex) {
  const [r, g, b] = parseHex(hex).map(toLinear)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

export function contrast(one, two) {
  const a = luminance(one)
  const b = luminance(two)
  return round((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05), 2)
}

export const round = (value, places = 3) => Number(value.toFixed(places))

/** The same colour, moved along the ladder. */
export const lighten = (color, by) => ({ ...color, l: round(clamp(color.l + by), 4) })

/** The same colour with less of itself, for tints and soft text. */
export const desaturate = (color, factor) => ({ ...color, c: round(color.c * factor, 4) })

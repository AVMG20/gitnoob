import { invoke } from './useInvoke'

/**
 * Web links open in the browser.
 *
 * Markdown from reviews and the links in Settings carry `target="_blank"`,
 * which a browser turns into a new tab and the app's webview turns into
 * nothing at all: there is no second window to put the page in. So a click on
 * any http(s) link is handed to the desktop instead, the same way the review
 * pane's "open in browser" works.
 */
export function onLinkClick(event: MouseEvent) {
  if (event.defaultPrevented || event.button !== 0) return
  const target = event.target
  if (!(target instanceof Element)) return
  const link = target.closest('a[href]')
  if (!link) return
  const href = link.getAttribute('href') ?? ''
  if (!/^https?:\/\//i.test(href)) return
  event.preventDefault()
  void invoke('open_external', { url: href }).catch(() => undefined)
}

/** Starts sending web links to the browser; the returned function stops it. */
export function installExternalLinks() {
  document.addEventListener('click', onLinkClick)
  return () => document.removeEventListener('click', onLinkClick)
}

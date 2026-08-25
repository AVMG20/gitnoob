import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { computed, reactive } from 'vue'

/**
 * Updating from the releases the tag pipeline builds.
 *
 * The plugin asks GitHub for `latest.json`, compares its version against the
 * one compiled into this app, and downloads a bundle whose signature has to
 * verify against the public key in `tauri.conf.json` before anything is
 * written. So the trust here is in the key, not in the URL: a release that
 * GitHub served but the signing key never touched is refused.
 */
export type UpdateStage =
  | 'idle'
  | 'checking'
  | 'none'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'error'

/**
 * The handle itself is kept out of the reactive store. It is a class instance
 * holding a resource id, and wrapping one in a proxy is how you lose the
 * methods on it.
 */
let pending: Update | null = null

const store = reactive({
  stage: 'idle' as UpdateStage,
  /** This app's own version, read once from the bundle. */
  current: '',
  /** The version on offer, when there is one. */
  version: null as string | null,
  /** The release notes GitHub was given, as markdown. */
  notes: null as string | null,
  date: null as string | null,
  downloaded: 0,
  /** Zero when the server did not say how large the download is. */
  total: 0,
  error: null as string | null,
  /** When the last check finished, so the page can say "no, just now". */
  checked: null as number | null
})

/** The date the updater reports, `2026-08-25 09:14:02.0 +00:00:00`, trimmed. */
function readDate(raw: string | undefined): string | null {
  if (!raw) return null
  const parsed = new Date(raw.replace(/\.\d+\s/, ' ').replace(/\s\+00:00:00$/, 'Z'))
  return Number.isNaN(parsed.getTime()) ? raw : parsed.toLocaleDateString()
}

export function useUpdates() {
  const busy = computed(() => store.stage === 'checking' || store.stage === 'downloading')
  const progress = computed(() =>
    store.total ? Math.min(100, Math.round((store.downloaded / store.total) * 100)) : 0
  )

  async function version() {
    if (!store.current) store.current = await getVersion().catch(() => '')
    return store.current
  }

  /**
   * Asks whether there is a newer release.
   *
   * `quiet` is for the check at launch: a machine that is offline, or behind a
   * proxy that eats the request, should not be told about it every time the
   * window opens. The button in settings passes nothing and does report.
   */
  async function checkForUpdate(quiet = false) {
    if (busy.value) return
    store.stage = 'checking'
    store.error = null
    await version()
    try {
      const found = await check()
      store.checked = Date.now()
      pending = found
      if (!found) {
        store.stage = 'none'
        store.version = null
        return null
      }
      store.version = found.version
      store.notes = found.body ?? null
      store.date = readDate(found.date)
      store.stage = 'available'
      return found
    } catch (error) {
      pending = null
      if (quiet) {
        store.stage = 'idle'
        return null
      }
      store.error = String(error)
      store.stage = 'error'
      return null
    }
  }

  /**
   * Downloads and installs, then restarts into it.
   *
   * On Windows the installer closes the app itself, so the relaunch below is
   * never reached there — which is why the restart is not a separate button the
   * user has to find afterwards.
   */
  async function install() {
    if (!pending || busy.value) return
    store.stage = 'downloading'
    store.downloaded = 0
    store.total = 0
    store.error = null
    try {
      await pending.downloadAndInstall((event) => {
        if (event.event === 'Started') store.total = event.data.contentLength ?? 0
        else if (event.event === 'Progress') store.downloaded += event.data.chunkLength
        else if (event.event === 'Finished') store.downloaded = store.total || store.downloaded
      })
      store.stage = 'ready'
      await relaunch()
    } catch (error) {
      store.error = String(error)
      store.stage = 'error'
    }
  }

  function dismiss() {
    if (store.stage === 'available') store.stage = 'idle'
  }

  return { store, busy, progress, version, checkForUpdate, install, dismiss }
}

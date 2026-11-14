import { watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from './useInvoke'
import { prepareToQuit, saveStates } from './useFileEdit'

/**
 * The window's half of quitting (the backend's is quit.rs).
 *
 * The close button, ⌘Q, the Dock's Quit, logout and shutdown all end up in
 * `leaveApp`: everything is saved, anything that cannot be is asked about,
 * and the window closes itself, which ends the app. The backend only hands a
 * quit over while this side has said it is listening (`quit_ready`) and that
 * it holds edits not on disk (`quit_guard`); otherwise the app just goes.
 *
 * Wired up before anything else when the window starts, so a quit pressed
 * while a repository is still opening is not lost.
 */

let leaving = false
/** A quit being prepared: a second press while it runs is the same quit. */
let preparing = false
let unlisten: UnlistenFn[] = []
let stopGuard: (() => void) | null = null

/** Saves, asks if it must, and closes the window. Once at a time. */
export async function leaveApp() {
  if (leaving || preparing) return
  preparing = true
  try {
    if (!(await prepareToQuit())) return
    leaving = true
    await getCurrentWindow().destroy()
  } finally {
    preparing = false
  }
}

const UNSAVED = new Set(['unsaved', 'saving', 'conflict', 'failed'])

export async function watchQuit() {
  try {
    const win = getCurrentWindow()
    unlisten.push(
      await win.onCloseRequested(async (event) => {
        if (leaving) return
        event.preventDefault()
        await leaveApp()
      })
    )
    unlisten.push(
      await listen<number>('quit-requested', ({ payload }) => {
        void invoke('quit_heard', { ticket: payload }).catch(() => undefined)
        void leaveApp()
      })
    )
  } catch {
    // Not in a Tauri window (the dev server in a browser): nothing to hold.
    return
  }
  stopGuard = watch(
    () => Object.values(saveStates).some((state) => UNSAVED.has(state)),
    (needed) => void invoke('quit_guard', { needed }).catch(() => undefined),
    { immediate: true }
  )
  await invoke('quit_ready').catch(() => undefined)
}

export function unwatchQuit() {
  for (const off of unlisten) off()
  unlisten = []
  stopGuard?.()
  stopGuard = null
}

/** For tests. */
export function resetQuit() {
  unwatchQuit()
  leaving = false
  preparing = false
}

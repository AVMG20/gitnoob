import { invoke as send } from '@tauri-apps/api/core'

/**
 * Every call to the backend, addressed to the repository it is about.
 *
 * The window has project tabs; the backend holds one open path. Without saying
 * which repository a call means, a tab switched while a slow operation ran
 * moved the path out from under it, and the rest of that operation acted on the
 * other repository — a race with nothing on screen to explain it.
 *
 * So each call carries `__repo`, and the backend applies it before running the
 * command. It is set here rather than at each of the two hundred call sites,
 * because a call site that forgot would be exactly the bug this closes.
 */
let target: string | null = null

/** Names the repository every following call is about. */
export function aimAt(path: string | null) {
  target = path
}

/** Which repository calls are currently addressed to. */
export function aimedAt() {
  return target
}

export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  // A call made before any repository is open — settings, profiles, the model
  // list — is addressed to nothing, and the backend leaves the path alone.
  return send<T>(command, target === null ? args : { ...(args ?? {}), __repo: target })
}

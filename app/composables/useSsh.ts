import { invoke } from '@tauri-apps/api/core'
import { reactive } from 'vue'

export interface SshKey {
  path: string
  name: string
  kind: string
  comment: string
}

export interface SshTest {
  ok: boolean
  message: string
  user: string | null
}

const store = reactive({
  keys: [] as SshKey[],
  loaded: false,
  testing: false,
  /** The last test result, so the form can keep showing it. */
  result: null as SshTest | null
})

export function useSsh() {
  /** Reads `~/.ssh` once per settings visit; keys rarely appear mid-session. */
  async function loadKeys(force = false) {
    if (store.loaded && !force) return store.keys
    store.keys = await invoke<SshKey[]>('ssh_keys').catch(() => [])
    store.loaded = true
    return store.keys
  }

  async function test(host: string | null, key: string | null) {
    store.testing = true
    store.result = null
    try {
      store.result = await invoke<SshTest>('ssh_test', { host, key })
    } catch (error) {
      store.result = { ok: false, message: String(error), user: null }
    } finally {
      store.testing = false
    }
    return store.result
  }

  function clear() {
    store.result = null
  }

  return { store, loadKeys, test, clear }
}

/** "id_ed25519 — ssh-ed25519, you@example.com", or just the name if bare. */
export function describeKey(key: SshKey): string {
  const parts = [key.kind, key.comment].filter(Boolean)
  return parts.length ? `${key.name} — ${parts.join(', ')}` : key.name
}

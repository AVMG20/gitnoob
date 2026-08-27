import { execFileSync } from 'node:child_process'
import { beforeEach, describe, expect, it, vi } from 'vitest'

/** What the wrapper handed to Tauri, since the Tauri host is not here. */
const sent: { command: string; args: unknown }[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args: unknown) => {
    sent.push({ command, args })
    return Promise.resolve(null)
  }
}))

/**
 * Every call to the backend goes through `useInvoke`.
 *
 * The window has project tabs while the backend holds one open path, so each
 * call says which repository it is about — `useInvoke` stamps `__repo` on it and
 * the backend applies that before the command runs. A call that imports
 * `invoke` straight from Tauri skips the stamp, is addressed to nothing, and
 * acts on whichever repository happens to be open when it lands. That is the
 * race this whole arrangement exists to close, and it comes back the moment one
 * import is written the old way.
 *
 * Nothing in the language stops that, so this does. It is the same shape as the
 * check that keeps hardcoded colours out of the components.
 */
describe('the backend call wrapper', () => {
  beforeEach(() => {
    sent.length = 0
  })

  it('is the only thing importing invoke from Tauri', () => {
    const found = execFileSync('sh', [
      '-c',
      "grep -rln \"from '@tauri-apps/api/core'\" app || true"
    ])
      .toString()
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
      // The wrapper itself is where the real import belongs.
      .filter((path) => !path.endsWith('composables/useInvoke.ts'))

    expect(
      found,
      `import { invoke } from '~/composables/useInvoke' instead — a call that ` +
        `skips the wrapper is not addressed to any repository`
    ).toEqual([])
  })

  it('stamps the repository a call is about, and only once one is open', async () => {
    const { invoke, aimAt } = await import('../app/composables/useInvoke')

    // Before a repository is open, a call carries nothing extra: settings and
    // profiles are not about a repository, and stamping an empty one would ask
    // the backend to open "".
    aimAt(null)
    await invoke('config_read')
    expect(sent.at(-1)).toEqual({ command: 'config_read', args: undefined })

    aimAt('/repos/thing')
    await invoke('working_status')
    expect(sent.at(-1)).toEqual({
      command: 'working_status',
      args: { __repo: '/repos/thing' }
    })

    // The command's own arguments survive alongside it.
    await invoke('commit_graph', { limit: 500 })
    expect(sent.at(-1)).toEqual({
      command: 'commit_graph',
      args: { limit: 500, __repo: '/repos/thing' }
    })

    // Closing the last tab means the next call is about nothing again.
    aimAt(null)
    await invoke('config_read')
    expect(sent.at(-1)).toEqual({ command: 'config_read', args: undefined })
  })
})

// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ActivityLog from '~/components/ActivityLog.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const git = useGit()

function answer(out: Partial<{ ok: boolean; code: number; stdout: string; stderr: string }>) {
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'run_git') return { argv: [], ok: true, code: 0, stdout: '', stderr: '', ...out }
    return null
  })
}

async function type(wrapper: ReturnType<typeof mount>, line: string) {
  const input = wrapper.find('input')
  await input.setValue(line)
  await wrapper.find('form').trigger('submit')
  await flushPromises()
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  git.store.log.length = 0
  git.store.repo = { path: '/tmp/repo', name: 'repo', head: 'main', detached: false } as never
})

describe('the console', () => {
  it('reads oldest first, with the prompt beneath the transcript', async () => {
    answer({ stdout: 'abc1234 first\n' })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'log --oneline -1')

    const rows = wrapper.findAll('.entry .text').map((one) => one.text())
    const command = rows.findIndex((text) => text === 'git log --oneline -1')
    const output = rows.findIndex((text) => text === 'abc1234 first')
    expect(command).toBeGreaterThanOrEqual(0)
    expect(output).toBe(command + 1)

    // The input is the last thing in the console, under everything else.
    const order = [...wrapper.element.querySelectorAll('.body, .prompt-row')]
    expect(order[0]!.className).toContain('body')
    expect(order[1]!.className).toContain('prompt-row')
  })


  it('opens with the terminal button, caret in the prompt', async () => {
    const wrapper = mount(ActivityLog, { attachTo: document.body })
    expect(wrapper.find('input').exists()).toBe(false)
    await wrapper.find('.term').trigger('click')
    expect(wrapper.find('input').exists()).toBe(true)
    expect(document.activeElement).toBe(wrapper.find('input').element)
    wrapper.unmount()
  })

  it('runs what was typed and writes the command and its output to the log', async () => {
    answer({ stdout: 'abc1234 first\n' })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'git log --oneline -1')

    expect(calls.find((c) => c.cmd === 'run_git')?.args.args).toEqual(['log', '--oneline', '-1'])
    const lines = git.store.log.map((l) => [l.level, l.text])
    expect(lines).toContainEqual(['command', 'git log --oneline -1'])
    expect(lines).toContainEqual(['output', 'abc1234 first'])
    expect(wrapper.find('input').element.value).toBe('')
  })

  it('shows a failed command in red, with what git said, and no notice', async () => {
    answer({ ok: false, code: 128, stderr: "fatal: not a valid object name: 'nope'" })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'show nope')

    // The store is newest-first; the console turns it round for reading, so
    // what the user sees is the command with its output beneath it.
    expect(git.store.log[0]).toMatchObject({ level: 'output' })
    expect(git.store.log[1]).toMatchObject({ level: 'failed', text: 'git show nope' })
    expect(wrapper.find('.entry.failed').exists()).toBe(true)
  })

  it('recalls earlier lines with the arrow keys', async () => {
    answer({})
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'status')
    await type(wrapper, 'fetch')

    const input = wrapper.find('input')
    await input.trigger('keydown', { key: 'ArrowUp' })
    expect(input.element.value).toBe('fetch')
    await input.trigger('keydown', { key: 'ArrowUp' })
    expect(input.element.value).toBe('status')
    await input.trigger('keydown', { key: 'ArrowDown' })
    await input.trigger('keydown', { key: 'ArrowDown' })
    expect(input.element.value).toBe('')
  })

  it('has nothing to type into without a repository', async () => {
    git.store.repo = null
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    expect(wrapper.find('input').attributes('disabled')).toBeDefined()
  })
})

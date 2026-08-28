// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import CommitDetails from '~/components/CommitDetails.vue'
import Avatar from '~/components/Avatar.vue'
import { useGit, type CommitSignature } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

let signature: CommitSignature | null = null

beforeEach(() => {
  signature = null
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'commit_signature') return signature
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.detail = {
    oid: 'a91c4e2000000000000000000000000000000000',
    short: 'a91c4e2',
    summary: 'fix(export): keep the header row on page two',
    body: '',
    author: 'Ramon Robben',
    email: 'ramon@example.com',
    committer: 'Ramon Robben',
    time: 1756000000,
    parents: [],
    files: []
  } as never
})

const show = () => mount(CommitDetails, { global: { components: { Avatar }, stubs: { Avatar: true, Spinner: true } } })

describe('the signature line in the commit details', () => {
  it('is absent for a commit nobody signed', async () => {
    signature = { verdict: 'none', signer: null, key: null, fingerprint: null, raw: null }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.verline').exists()).toBe(false)
  })

  it('names the signer, in the colour of what the signature is worth', async () => {
    signature = {
      verdict: 'good',
      signer: 'Ramon Robben',
      key: 'SHA256:0mB',
      fingerprint: null,
      raw: 'Good "git" signature'
    }
    const wrapper = show()
    await flushPromises()
    const line = wrapper.find('.verline')
    expect(line.classes()).toContain('good')
    expect(line.text()).toContain('Signed by Ramon Robben')
  })

  it('calls a bad signature bad, in red', async () => {
    signature = { verdict: 'bad', signer: 'A Contributor', key: null, fingerprint: null, raw: 'BAD signature' }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.verline').classes()).toContain('bad')
    expect(wrapper.text()).toContain('not what was signed')
  })

  it("keeps git's own words folded away until asked", async () => {
    signature = {
      verdict: 'good',
      signer: 'Ramon Robben',
      key: 'SHA256:0mB',
      fingerprint: 'FINGER',
      raw: 'gpg: Good signature from "Ramon"'
    }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.verbody').exists()).toBe(false)
    await wrapper.find('.verline').trigger('click')
    expect(wrapper.find('.raw').text()).toContain('Good signature')
    expect(wrapper.find('.verbody').text()).toContain('SHA256:0mB')
  })

  it('does not offer to unfold a signature git said nothing more about', async () => {
    signature = { verdict: 'unchecked', signer: null, key: null, fingerprint: null, raw: null }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.verline').attributes('disabled')).toBeDefined()
  })
})

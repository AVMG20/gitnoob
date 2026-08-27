import { describe, expect, it } from 'vitest'
import { explain } from '~/composables/gitErrors'

/**
 * What the notices say. Every string here is one git actually writes, kept
 * whole, because the rules are matched against the paragraph and not against a
 * tidied version of it that nobody would ever be shown.
 */
describe('explaining a git failure', () => {
  it('turns a refused branch switch into what to do about it', () => {
    const raw = [
      'Checkout: error: Your local changes to the following files would be overwritten by checkout:',
      '\tapp/app.vue',
      'Please commit your changes or stash them before you switch branches.',
      'Aborting'
    ].join('\n')
    const said = explain(raw)
    expect(said.title).toBe(
      'You have open changes that this would overwrite. Commit or stash them first.'
    )
    // Nothing is lost: the files git named are still there to look at.
    expect(said.detail).toContain('app/app.vue')
  })

  it('tells untracked files apart from changed ones', () => {
    const said = explain(
      'error: The following untracked working tree files would be overwritten by checkout:\n\tnotes.md'
    )
    expect(said.title).toBe('Untracked files are in the way. Move, delete, or commit them first.')
  })

  it('says what a rejected push needs', () => {
    const said = explain(
      'Push: ! [rejected] main -> main (fetch first)\nerror: failed to push some refs'
    )
    expect(said.title).toBe('The remote has commits you have not got. Pull first, then push again.')
  })

  it('names the key when the remote refuses it', () => {
    const said = explain('Pull: git@github.com: Permission denied (publickey).')
    expect(said.title).toBe('The remote refused your SSH key. Check which key this profile pins.')
  })

  it('reads a lock as another process, not as a broken repository', () => {
    const said = explain(
      "Commit: fatal: Unable to create '/repo/.git/index.lock': File exists."
    )
    expect(said.title).toContain('Another git process')
  })

  it('says nothing about a merge stopping, since the resolver opens itself', () => {
    const said = explain('Merge: Auto-merging app.vue\nCONFLICT (content): Merge conflict in app.vue\nAutomatic merge failed; fix conflicts and then commit the result.')
    expect(said.quiet).toBe(true)
  })

  it('hands back anything it does not recognise, whole', () => {
    const raw = 'Rebase: fatal: something nobody has hit yet\nwith a second line of it'
    const said = explain(raw)
    expect(said.title).toBe('Rebase: fatal: something nobody has hit yet')
    expect(said.detail).toBe(raw)
  })

  it('leaves a one-line message without a detail to open', () => {
    const said = explain('Name did not match; nothing was deleted')
    expect(said.title).toBe('Name did not match; nothing was deleted')
    expect(said.detail).toBeNull()
  })

  it('has something to say about nothing at all', () => {
    expect(explain('   ').title).toBe('Something went wrong')
  })
})

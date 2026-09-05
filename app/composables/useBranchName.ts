/**
 * What is wrong with a branch name, said before git is asked.
 *
 * The rules git itself enforces, checked up front so the answer is inline
 * rather than an error after the fact. `null` means the name is fine. `taken`
 * is whether a local branch already has it, which only the caller knows.
 */
export function branchNameProblem(raw: string, taken: boolean): string | null {
  const name = raw.trim()
  if (!name) return 'Give the branch a name.'
  if (taken) return 'A branch with that name already exists.'
  if (
    /[\s~^:?*[\\]/.test(name) ||
    name.startsWith('-') ||
    name.startsWith('/') ||
    name.endsWith('/') ||
    name.endsWith('.') ||
    name.endsWith('.lock') ||
    name.includes('..') ||
    name.includes('@{') ||
    name.includes('//') ||
    name === '@'
  ) {
    return 'Git will not accept that name.'
  }
  return null
}

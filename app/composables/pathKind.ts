/**
 * What kind of file a path is, for ordering and colouring lists: a test, a
 * dependency (what Composer or npm installed), or a project file.
 *
 * Tests and dependencies are seldom where someone wants to go from a list of
 * usages or a search, so they sort after the project's files, dependencies
 * last of all. `files::is_test_path` in the backend says the same about tests.
 */

export type PathKind = 'project' | 'test' | 'vendor'

/** A path inside `vendor/` or `node_modules/` at the root of the work tree. */
export const DEPENDENCY = /^(vendor|node_modules)\//

const TEST_DIRS = new Set(['tests', 'test', '__tests__', 'spec', 'e2e'])

/**
 * A test file: under a `tests`, `Test` (Magento's `Test/Unit`), `__tests__`,
 * `spec` or `e2e` folder, or named the way a test runner finds tests
 * (`FooTest.php`, `foo.test.ts`, `foo.spec.js`, `foo_test.go`, `test_foo.py`).
 */
export function isTestPath(path: string): boolean {
  const parts = path.split('/')
  const name = parts.pop() ?? ''
  if (parts.some((dir) => TEST_DIRS.has(dir.toLowerCase()))) return true
  return (
    (name.endsWith('Test.php') && name.length > 'Test.php'.length) ||
    /.\.(test|spec)\.[^.]+$/i.test(name) ||
    /_test\.(go|py)$/i.test(name) ||
    /^test_.+\.py$/i.test(name)
  )
}

export function pathKind(path: string): PathKind {
  if (DEPENDENCY.test(path)) return 'vendor'
  if (isTestPath(path)) return 'test'
  return 'project'
}

const RANK: Record<PathKind, number> = { project: 0, test: 1, vendor: 2 }

/** Where a path goes in a list: project files, then tests, then dependencies. */
export function pathRank(path: string): number {
  return RANK[pathKind(path)]
}

/** The same list, tests after the project's files and dependencies last; otherwise as it was. */
export function testsLast<T extends { path: string }>(items: T[]): T[] {
  return items
    .map((item, order) => ({ item, order, rank: pathRank(item.path) }))
    .sort((a, b) => a.rank - b.rank || a.order - b.order)
    .map(({ item }) => item)
}

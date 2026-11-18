import { describe, expect, it } from 'vitest'
import { isTestPath, pathKind, testsLast } from '~/composables/pathKind'

describe('what kind of file a path is', () => {
  it('tells tests by their folder or their name, the same way the backend does', () => {
    for (const path of [
      'tests/Feature/InvoiceTest.php',
      'app/code/Acme/Shop/Test/Unit/Model/ProductTest.php',
      'dev/tests/integration/x.php',
      'src/__tests__/a.ts',
      'app/Billing/InvoiceTest.php',
      'src/a.test.ts',
      'src/a.spec.js',
      'pkg/a_test.go',
      'lib/test_a.py'
    ]) {
      expect(isTestPath(path), path).toBe(true)
    }
    for (const path of [
      'app/Latest.php',
      'app/Test.php',
      'src/contest.ts',
      '.test.ts',
      'src/a.test.ts.bak.d',
      'app/Testing/Helper.php',
      'lib/test_.py'
    ]) {
      expect(isTestPath(path), path).toBe(false)
    }
  })

  it('takes vendor/ and node_modules/ at the root as dependencies, before anything else', () => {
    expect(pathKind('vendor/acme/kit/tests/FooTest.php')).toBe('vendor')
    expect(pathKind('node_modules/x/index.js')).toBe('vendor')
    expect(pathKind('app/vendor/x.php')).toBe('project')
    expect(pathKind('tests/a.php')).toBe('test')
  })

  it('sorts tests after the rest and dependencies last, keeping the order within each', () => {
    const sorted = testsLast([
      { path: 'vendor/a.php' },
      { path: 'tests/b.php' },
      { path: 'app/c.php' },
      { path: 'tests/d.php' },
      { path: 'app/e.php' }
    ])
    expect(sorted.map((one) => one.path)).toEqual(['app/c.php', 'app/e.php', 'tests/b.php', 'tests/d.php', 'vendor/a.php'])
  })
})

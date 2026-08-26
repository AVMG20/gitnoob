import { describe, expect, it } from 'vitest'
import { labelFor, languageFor } from '../app/composables/useHighlight'

describe('languageFor', () => {
  it('reads an ordinary extension', () => {
    expect(languageFor('src/app.ts')).toBe('typescript')
    expect(languageFor('Makefile')).toBe('makefile')
  })

  it('colours a dotfile rather than treating its name as an extension', () => {
    expect(languageFor('.gitignore')).toBe('bash')
    expect(languageFor('.editorconfig')).toBe('ini')
    expect(languageFor('.prettierrc')).toBe('json')
  })

  it('takes a dotfile suffix as a variant, not as a type', () => {
    expect(languageFor('.env.local')).toBe('ini')
    expect(languageFor('.env.production')).toBe('ini')
  })

  it('still prefers a real extension on a dotfile that has one', () => {
    expect(languageFor('.eslintrc.json')).toBe('json')
    expect(languageFor('.eslintrc.yml')).toBe('yaml')
  })

  it('matches a named file whatever its case', () => {
    expect(languageFor('Dockerfile')).toBe('dockerfile')
    expect(languageFor('deploy/DOCKERFILE')).toBe('dockerfile')
    expect(languageFor('CMakeLists.txt')).toBe('cmake')
  })

  it('reads a file inside a directory by the file, not the path', () => {
    expect(languageFor('some.dir/app')).toBeNull()
    expect(languageFor('some.dir/app.rs')).toBe('rust')
  })

  it('gives back null for something it has no grammar for', () => {
    expect(languageFor('binary.bin')).toBeNull()
    expect(languageFor('LICENSE')).toBeNull()
  })
})

describe('labelFor', () => {
  it('names a file by what it is, not by the grammar used on it', () => {
    // Painted with the xml grammar, but nobody calls a Vue file xml.
    expect(labelFor('App.vue')).toBe('vue')
    expect(labelFor('main.rs')).toBe('rust')
    expect(labelFor('a.cpp')).toBe('c++')
  })

  it('names a dotfile after itself', () => {
    expect(labelFor('.gitignore')).toBe('.gitignore')
    expect(labelFor('.env.local')).toBe('.env')
  })

  it('has nothing to say about a file with no extension it knows', () => {
    expect(labelFor('LICENSE')).toBeNull()
  })
})

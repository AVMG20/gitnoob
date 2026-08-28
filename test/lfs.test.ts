import { describe, expect, it } from 'vitest'
import { humanSize, readPointer } from '~/composables/useLfs'

const POINTER = [
  'version https://git-lfs.github.com/spec/v1',
  'oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393',
  'size 12345',
  ''
].join('\n')

describe('reading an LFS pointer', () => {
  it('reads the object and the size out of one', () => {
    expect(readPointer(POINTER)).toEqual({
      oid: 'sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393',
      size: 12345
    })
  })

  it('says nothing about an ordinary file', () => {
    expect(readPointer('const x = 1\n')).toBeNull()
    expect(readPointer('')).toBeNull()
    expect(readPointer(null)).toBeNull()
  })

  it('is not fooled by a file that merely mentions the spec', () => {
    expect(readPointer('# see https://git-lfs.github.com/spec/v1\noid x\nsize 2\n')).toBeNull()
  })

  it('refuses one missing half of itself', () => {
    expect(readPointer('version https://git-lfs.github.com/spec/v1\noid sha256:aa\n')).toBeNull()
    expect(readPointer('version https://git-lfs.github.com/spec/v1\nsize 3\n')).toBeNull()
  })

  it('refuses a size that is not one', () => {
    expect(
      readPointer('version https://git-lfs.github.com/spec/v1\noid sha256:aa\nsize huge\n')
    ).toBeNull()
  })
})

describe('saying how big a file is', () => {
  it('counts in the unit a person would', () => {
    expect(humanSize(0)).toBe('0 B')
    expect(humanSize(999)).toBe('999 B')
    expect(humanSize(1024)).toBe('1 KB')
    expect(humanSize(1536)).toBe('1.5 KB')
    expect(humanSize(12 * 1024 * 1024)).toBe('12 MB')
    expect(humanSize(3.5 * 1024 * 1024 * 1024)).toBe('3.5 GB')
  })
})

import { aimedAt, invokeAt } from './useInvoke'

/**
 * Images in the viewer: which files are tried as one, and reading one.
 *
 * The extension only decides whether to try. The backend sniffs the bytes
 * and says what they are; anything it does not recognise is not shown. The
 * bytes become a Blob URL, which whoever asked revokes when it is done, and
 * are only ever drawn through `<img>`, so an SVG's scripts never run.
 */

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'svg'])

function extensionOf(path: string) {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  return name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
}

export const isImagePath = (path: string) => IMAGE_EXTENSIONS.has(extensionOf(path))
export const isSvgPath = (path: string) => extensionOf(path) === 'svg'

/** Where an image is read from: the disk, the index, HEAD, a commit or its parent. */
export type ImageSource = 'work' | 'index' | 'head' | 'commit' | 'parent'

export interface ImageSide {
  source: ImageSource
  commit?: string
}

export interface ImageRead {
  mime: string | null
  size: number
  too_large: boolean
  /** The pointer's text, when the file is a Git LFS pointer. */
  lfs: string | null
  missing: boolean
  /** Base64. */
  data: string | null
}

export interface LoadedImage extends ImageRead {
  /** A Blob URL for `<img>`; revoke it with `dropImage`. */
  url: string | null
}

/** Reads one side of an image and makes a Blob URL of it. */
export async function readImage(path: string, side: ImageSide, repo?: string | null): Promise<LoadedImage> {
  const read = await invokeAt<ImageRead>(repo === undefined ? aimedAt() : repo, 'image_read', {
    path,
    source: side.source,
    commit: side.commit ?? null
  })
  let url: string | null = null
  if (read?.data && read.mime) {
    const binary = atob(read.data)
    const bytes = new Uint8Array(binary.length)
    for (let at = 0; at < binary.length; at++) bytes[at] = binary.charCodeAt(at)
    url = URL.createObjectURL(new Blob([bytes], { type: read.mime }))
  }
  return { ...read, data: null, url }
}

export function dropImage(image: LoadedImage | null) {
  if (image?.url) URL.revokeObjectURL(image.url)
}

/** PNG, JPEG, SVG… from a sniffed type. */
export function formatOf(mime: string | null) {
  if (!mime) return ''
  const kind = mime.replace(/^image\//, '').replace(/\+xml$/, '').replace(/^x-icon$/, 'ico')
  return kind.toUpperCase()
}

export function bytesLabel(size: number) {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}

/** What the header says about an image on screen. */
export interface ImageInfo {
  width: number
  height: number
  size: number
  mime: string | null
}

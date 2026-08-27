<script setup lang="ts">
import { onMounted } from 'vue'
import ConflictOverlay from './ConflictOverlay.vue'
import ContextMenu from './ContextMenu.vue'
import { useGit, type ConflictBlock, type Resolution } from '~/composables/useGit'

/**
 * The conflict resolver on a fixture merge, for looking at it in a browser.
 *
 * Reached with `?lab=conflict` on the dev server and compiled out of anything
 * built for release. A real conflict needs a repository mid-merge, which is a
 * slow and destructive thing to arrange every time a colour or a row height is
 * being judged — so the backend commands the resolver uses are answered here
 * instead, from the tables below. The rendering of the file is the same code
 * the app runs; only the file is invented.
 */
const git = useGit()

const ours = 'HEAD'
const theirs = 'feature/call-of-xeno-xenoray-damage-buff'

/** A run of plausible context, so scrolling and the strip have something to do. */
function filler(from: number, count: number): string {
  return Array.from(
    { length: count },
    (_, at) => `    const value${from + at} = compute(${from + at}, state.frame)`
  ).join('\n')
}

const region = (index: number, our: string[], their: string[]): ConflictBlock => ({
  kind: 'conflict',
  index,
  ours: our,
  theirs: their,
  base: [],
  has_base: index % 3 === 0,
  ours_label: ours,
  theirs_label: theirs
})

const context = (text: string): ConflictBlock => ({ kind: 'context', lines: text.split('\n') })

const BLOCKS: ConflictBlock[] = [
  context(
    [
      'import { computed, ref } from "vue"',
      'import { useGame } from "~/composables/useGame"',
      '',
      'const state = useGame()',
      filler(1, 14)
    ].join('\n')
  ),
  region(
    0,
    ['const damage = base * 1.0', 'const crit = 2.0'],
    ['const damage = base * 1.35', 'const crit = 2.5', 'const bleed = 0.2']
  ),
  context(filler(20, 22)),
  region(
    1,
    ['  hurtVeil.value = 0.4', '  shake(6)', '  play("hit")'],
    ['  hurtVeil.value = 0.65']
  ),
  context(filler(50, 9)),
  region(2, ['const reserves = 0'], ['const reserves = metaBalance']),
  context(filler(62, 30)),
  region(
    3,
    [],
    ['function onXenoray(mark: Mark) {', '  marks.push(mark)', '}']
  ),
  context(filler(95, 18)),
  region(4, ['  return points * 1.0', '}'], ['  return points * 1.25 + streak', '}']),
  context(filler(120, 26)),
  region(
    5,
    ['const label = "Workbench // Equipment"', 'const icon = "i-lucide-wrench"'],
    ['const label = "Loadout"', 'const icon = "i-lucide-trophy"']
  ),
  context(filler(150, 12)),
  region(6, ['  guestMode.value = true'], []),
  context(filler(165, 20)),
  region(
    7,
    ['export const version = "0.4.0"'],
    ['export const version = "0.5.0-xenoray"', 'export const channel = "beta"']
  ),
  context(filler(190, 24))
]

/** The delete/modify case: no markers in the file, the conflict all in index. */
const STAGES: Record<string, { base: boolean; ours: boolean; theirs: boolean }> = {
  'shared/utils/gamelogic/call-of-xeno-save.ts': { base: true, ours: false, theirs: true }
}

const FILES: Record<string, ConflictBlock[]> = {
  'app/components/games/CallOfXenoGame.client.vue': BLOCKS,
  'shared/utils/gamelogic/call-of-xeno-map.ts': [
    context(filler(1, 12)),
    region(0, ['const tiles = 64'], ['const tiles = 96']),
    context(filler(20, 30))
  ],
  'shared/utils/gamelogic/call-of-xeno-save.ts': [context(filler(1, 40))],
  'test/games/call-of-xeno.spec.ts': [
    context(filler(1, 8)),
    region(0, ['expect(damage).toBe(10)'], ['expect(damage).toBe(13.5)']),
    context(filler(12, 14))
  ]
}

function read(path: string) {
  const blocks = FILES[path] ?? []
  return {
    path,
    blocks,
    conflict_count: blocks.filter((block) => block.kind === 'conflict').length,
    stages: STAGES[path] ?? { base: true, ours: true, theirs: true }
  }
}

/** The same walk the Rust side does, so the result pane is honest. */
function preview(path: string, choices: Resolution[]) {
  const out: string[] = []
  for (const block of FILES[path] ?? []) {
    if (block.kind === 'context') {
      out.push(...block.lines)
      continue
    }
    const choice = choices[block.index]
    if (choice?.custom) {
      out.push(...choice.custom)
      continue
    }
    const take = { ours: choice?.take_ours ?? true, theirs: choice?.take_theirs ?? false }
    const first = choice?.ours_first ?? true
    if (first) {
      if (take.ours) out.push(...block.ours)
      if (take.theirs) out.push(...block.theirs)
    } else {
      if (take.theirs) out.push(...block.theirs)
      if (take.ours) out.push(...block.ours)
    }
  }
  return out.join('\n')
}

/**
 * Answers the handful of commands the resolver sends.
 *
 * `invoke` goes through this hook, which is the seam the Tauri window fills in;
 * in a browser there is nothing there at all, so the resolver would show an
 * empty file and every button would quietly fail.
 */
function install() {
  const internals = ((window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ ??=
    {}) as Record<string, unknown>
  internals.invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
    if (cmd === 'conflict_read') return read(String(args.path))
    if (cmd === 'conflict_preview') {
      return preview(String(args.path), (args.choices ?? []) as Resolution[])
    }
    if (cmd === 'conflict_resolve' || cmd === 'conflict_resolve_whole') {
      const left = Object.keys(FILES).filter((name) => name !== String(args.path))
      delete FILES[String(args.path)]
      git.store.status = { staged: [], unstaged: [], conflicted: left }
      return `Resolved ${args.path}`
    }
    if (cmd === 'ai_status') return { configured: true, model: 'fixture', commit_style: 'plain' }
    return null
  }
}

onMounted(() => {
  install()
  git.store.status = { staged: [], unstaged: [], conflicted: Object.keys(FILES) }
  git.store.progress = {
    merging: true,
    rebasing: false,
    cherry_picking: false,
    reverting: false,
    restoring: false
  }
  git.store.resolving = Object.keys(FILES)[0] ?? null
})
</script>

<template>
  <ConflictOverlay />
  <ContextMenu />
</template>

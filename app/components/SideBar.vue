<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Archive,
  ArrowDown,
  ArrowUp,
  ChevronRight,
  Cloud,
  Copy,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  GitBranch,
  GitMerge,
  GitPullRequest,
  HardDrive,
  Hash,
  Pencil,
  Plus,
  Search,
  Tag,
  Trash2,
  Upload
} from 'lucide-vue-next'
import { copyText, relativeTime, useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { useForge } from '~/composables/useForge'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const drag = useDragDrop()
const forge = useForge()

const open = reactive({ locals: true, remotes: true, tags: false, stashes: true, reviews: true })
const filter = ref('')
/** The branch whose deletion is being confirmed. */
const deleting = ref<string | null>(null)
/** Set while the new pull request dialog is open. */
const creatingReview = ref(false)
const prompt = ref<{
  title: string
  label: string
  initial?: string
  hint?: string
  confirm: string
  danger?: boolean
  run: (value: string) => void
} | null>(null)

const match = (name: string) =>
  !filter.value.trim() || name.toLowerCase().includes(filter.value.trim().toLowerCase())

/**
 * Folders the user has shut. Everything starts open, so a window that has never
 * been touched shows branches rather than a wall of closed folders.
 */
const shut = reactive(new Set<string>())

function toggleFolder(path: string) {
  if (shut.has(path)) shut.delete(path)
  else shut.add(path)
}

/** A folder heading or a branch, at a depth, ready for one flat `v-for`. */
type Shelf<T> =
  | { kind: 'folder'; key: string; path: string; label: string; depth: number; shut: boolean }
  | { kind: 'branch'; key: string; item: T; label: string; depth: number }

/**
 * Sorts branch names into the folders their slashes already describe.
 *
 * `feature/login` and `feature/signup` are two branches in one place, and a
 * project that names them that way ends up with a column of near-identical
 * prefixes, each one pushing the part that differs off the right-hand edge. The
 * folder carries the shared half, and a branch is listed under it by the part
 * that tells it apart.
 *
 * The list is flat, with a depth on each row, rather than a component nesting
 * itself: the rows are drag targets and menu targets, and keeping them siblings
 * keeps that wiring in one place.
 *
 * While a filter is typed every folder is open — someone searching wants what
 * they searched for, not the folder it happens to live in.
 */
function shelve<T extends { name: string }>(items: T[], scope: string): Shelf<T>[] {
  const rows: Shelf<T>[] = []
  const searching = !!filter.value.trim()
  const sorted = [...items].sort((a, b) => a.name.localeCompare(b.name))
  let trail: string[] = []
  // The depth of the shut folder currently hiding rows, if any.
  let hidden: number | null = null

  for (const item of sorted) {
    const parts = item.name.split('/')
    const dirs = parts.slice(0, -1)

    let same = 0
    while (same < dirs.length && same < trail.length && dirs[same] === trail[same]) same++
    if (hidden !== null && hidden >= same) hidden = null
    trail = trail.slice(0, same)

    for (let i = same; i < dirs.length; i++) {
      trail.push(dirs[i]!)
      if (hidden !== null) continue
      const path = `${scope}:${trail.join('/')}`
      const closed = !searching && shut.has(path)
      rows.push({ kind: 'folder', key: `folder:${path}`, path, label: dirs[i]!, depth: i, shut: closed })
      if (closed) hidden = i
    }

    if (hidden === null) {
      rows.push({
        kind: 'branch',
        key: `branch:${scope}:${item.name}`,
        item,
        label: parts[parts.length - 1]!,
        depth: dirs.length
      })
    }
  }
  return rows
}

const head = computed(() => store.repo?.head ?? '')
const locals = computed(() => (store.refs?.locals ?? []).filter((b) => match(b.name)))
const localShelf = computed(() => shelve(locals.value, 'local'))
const tags = computed(() => (store.refs?.tags ?? []).filter((t) => match(t.name)))
const stashes = computed(() => store.stashes)

const remoteGroups = computed(() => {
  const groups = new Map<string, { name: string; oid: string }[]>()
  for (const branch of store.refs?.remotes ?? []) {
    if (!match(branch.name)) continue
    const list = groups.get(branch.remote) ?? []
    list.push({ name: branch.name, oid: branch.oid })
    groups.set(branch.remote, list)
  }
  return [...groups.entries()].map(([remote, branches]) => ({ remote, branches }))
})

// --- drag and drop between branches

/**
 * Dropping one branch on another asks what to do with it.
 *
 * Merge and rebase are the same gesture with different consequences, so the
 * menu names both in terms of the two branches rather than in git's vocabulary.
 */
async function onDropOnBranch(event: MouseEvent, target: string, targetIsRemote: boolean) {
  const payload = drag.take(['branch', 'commit', 'stash'])
  if (!payload) return

  if (payload.kind === 'commit') {
    menu.show(
      event,
      [
        {
          label: `Cherry-pick ${payload.short} onto ${target}`,
          icon: GitBranch,
          hint: target === head.value ? '' : 'checks out first',
          action: async () => {
            if (target !== head.value) await git.checkout(target)
            await git.cherryPick([payload.oid])
          }
        }
      ],
      payload.summary.slice(0, 48)
    )
    return
  }

  if (payload.kind === 'stash') {
    menu.show(
      event,
      [
        {
          label: `Apply this stash on ${target}`,
          icon: Archive,
          action: async () => {
            if (target !== head.value) await git.checkout(target)
            await git.stashApply(payload.index)
          }
        }
      ],
      payload.message.slice(0, 48)
    )
    return
  }

  const source = payload.name
  if (source === target) return
  if (targetIsRemote) {
    git.note('Drop onto a local branch to merge or rebase', 'error')
    return
  }

  // What is actually possible between these two. Offering a move that would do
  // nothing is worse than not offering it: it gets picked, nothing visible
  // happens, and the user is left guessing whether it worked.
  const relation = await git.branchRelation(source, target)
  const ahead = relation?.ahead ?? 0
  const behind = relation?.behind ?? 0
  const merged = ahead === 0
  const fastForward = ahead > 0 && behind === 0

  if (merged) {
    menu.show(
      event,
      [
        {
          label: `${target} already has everything from ${source}`,
          icon: GitMerge,
          disabled: true
        }
      ],
      `${source} → ${target}`
    )
    return
  }

  menu.show(
    event,
    [
      // The cheap answer first, when there is one.
      ...(fastForward
        ? [
            {
              label: `Fast-forward ${source} → ${target}`,
              icon: GitMerge,
              hint: `${target} moves, no merge commit`,
              action: async () => {
                if (target !== head.value) await git.checkout(target)
                await git.merge(source, false)
              }
            }
          ]
        : []),
      {
        label: `Merge ${source} → ${target}`,
        icon: GitMerge,
        hint: fastForward ? `${target} changes, forced merge commit` : `${target} changes`,
        action: async () => {
          if (target !== head.value) await git.checkout(target)
          const outcome = await git.merge(source, fastForward)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      // Rebasing a branch that has nothing of its own is a fast-forward with
      // extra steps, so it is only worth offering once the two have diverged.
      ...(behind > 0
        ? [
            {
              label: `Rebase ${target} → onto ${source}`,
              icon: GitBranch,
              hint: `${behind} ${behind === 1 ? 'commit' : 'commits'} rewritten`,
              danger: true,
              action: async () => {
                if (target !== head.value) await git.checkout(target)
                const outcome = await git.rebase(source)
                if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
              }
            }
          ]
        : [])
    ],
    `${ahead} ahead · ${behind} behind`
  )
}

// --- context menus

function localMenu(event: MouseEvent, name: string, upstream: string | null) {
  const isHead = name === head.value
  menu.show(
    event,
    [
      // Ordered by how often it is wanted: the two everyday moves, then the
      // two that rewrite or combine history, then housekeeping, then the one
      // that destroys something.
      { label: 'Check out', icon: GitBranch, disabled: isHead, action: () => git.checkout(name) },
      {
        label: upstream ? `Push to ${upstream}` : 'Push and set upstream',
        icon: Upload,
        action: () => git.pushBranch(name, !upstream)
      },
      {
        // Works on a branch you are not standing on, open changes and all:
        // the backend moves the ref directly when it can, and visits the
        // branch and comes back when it cannot.
        label: upstream ? `Pull from ${upstream}` : 'Pull',
        icon: Download,
        disabled: !upstream,
        hint: upstream ? '' : 'no upstream',
        action: () => git.pullBranch(name)
      },
      { separator: true, label: '' },
      // Both of these move history between two branches, in opposite
      // directions, and "merge" alone does not say which way. Name both
      // branches and say which one ends up changed.
      {
        label: `Merge ${name} → ${head.value}`,
        icon: GitMerge,
        hint: `${head.value} changes`,
        disabled: isHead,
        action: async () => {
          const outcome = await git.merge(name, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: `Rebase ${head.value} → onto ${name}`,
        icon: GitBranch,
        hint: `${head.value} rewritten`,
        disabled: isHead,
        action: async () => {
          const outcome = await git.rebase(name)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      { separator: true, label: '' },
      {
        label: 'Copy branch name',
        icon: Copy,
        action: () => copyText(name, 'Branch')
      },
      {
        label: 'Rename…',
        icon: Pencil,
        action: () =>
          (prompt.value = {
            title: 'Rename branch',
            label: 'New name',
            initial: name,
            confirm: 'Rename',
            run: (value) => git.renameBranch(name, value)
          })
      },
      { separator: true, label: '' },
      {
        label: 'Delete branch…',
        icon: Trash2,
        danger: true,
        disabled: isHead,
        action: () => (deleting.value = name)
      }
    ],
    name
  )
}

function remoteMenu(event: MouseEvent, remote: string, name: string) {
  const full = `${remote}/${name}`
  menu.show(
    event,
    [
      {
        label: 'Check out as a local branch',
        icon: GitBranch,
        action: () => git.checkout(full)
      },
      {
        label: `Merge into ${head.value}`,
        icon: GitMerge,
        action: async () => {
          const outcome = await git.merge(full, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: 'Set as upstream of current branch',
        icon: Cloud,
        action: () => git.setUpstream(head.value, full)
      },
      { separator: true, label: '' },
      { label: 'Copy name', icon: Copy, action: () => copyText(full, 'Branch') },
      {
        label: `Delete ${name} on ${remote}`,
        icon: Trash2,
        danger: true,
        action: () =>
          (prompt.value = {
            title: `Delete ${name} on ${remote}?`,
            label: 'Type the branch name to confirm',
            confirm: 'Delete on remote',
            danger: true,
            hint: 'This removes the branch for everyone, not just here.',
            run: (value) => {
              if (value === name) git.deleteRemoteBranch(remote, name)
              else git.note('Name did not match; nothing was deleted', 'error')
            }
          })
      }
    ],
    full
  )
}

function tagMenu(event: MouseEvent, name: string, oid: string) {
  menu.show(
    event,
    [
      { label: 'Check out this tag', icon: GitBranch, action: () => git.checkout(name) },
      { label: 'Push tag to origin', icon: Upload, action: () => git.pushTag('origin', name) },
      { label: 'Copy tag name', icon: Copy, action: () => copyText(name, 'Tag') },
      { label: 'Copy commit hash', icon: Hash, action: () => copyText(oid, 'Hash') },
      { separator: true, label: '' },
      {
        label: 'Delete tag locally',
        icon: Trash2,
        danger: true,
        action: () => git.deleteTag(name)
      }
    ],
    name
  )
}

function stashMenu(event: MouseEvent, index: number, message: string) {
  menu.show(
    event,
    [
      { label: 'Apply and keep', icon: Archive, action: () => git.stashApply(index) },
      { label: 'Pop (apply and remove)', icon: Archive, action: () => git.stashPop(index) },
      { label: 'Show what is in it', icon: Search, action: () => git.selectStash(index) },
      {
        label: 'Turn into a branch…',
        icon: GitBranch,
        hint: 'safest',
        action: () =>
          (prompt.value = {
            title: 'Branch from stash',
            label: 'Branch name',
            confirm: 'Create branch',
            hint: 'Applies the stash onto a new branch and removes the entry.',
            run: (value) => git.stashBranch(index, value)
          })
      },
      { separator: true, label: '' },
      {
        label: 'Drop this stash',
        icon: Trash2,
        danger: true,
        action: () => git.stashDrop(index)
      }
    ],
    message
  )
}
</script>

<template>
  <aside class="side">
    <div class="filter">
      <Search :size="13" class="faint" />
      <input v-model="filter" type="search" placeholder="Filter branches" />
    </div>

    <div class="scroll">
      <!-- Local -->
      <button class="section-title toggle" @click="open.locals = !open.locals">
        <ChevronRight :size="12" class="chev" :class="{ down: open.locals }" />
        <HardDrive :size="12" class="mark" />
        Local
        <span class="count">{{ locals.length }}</span>
      </button>
      <div v-if="open.locals" class="group">
        <template v-for="row in localShelf" :key="row.key">
          <button
            v-if="row.kind === 'folder'"
            class="row folder"
            :style="{ paddingLeft: `calc(var(--indent) + ${row.depth * 14}px)` }"
            @click="toggleFolder(row.path)"
          >
            <ChevronRight :size="11" class="chev" :class="{ down: !row.shut }" />
            <component :is="row.shut ? Folder : FolderOpen" :size="13" class="glyph" />
            <span class="name truncate">{{ row.label }}</span>
          </button>
          <div
            v-else
            class="row"
            :class="{
              on: row.item.is_head,
              drop: drag.state.over === `branch:${row.item.name}`
            }"
            :style="{ paddingLeft: `calc(var(--indent) + ${row.depth * 14}px)` }"
            :title="`${row.item.name}${row.item.upstream ? ` → ${row.item.upstream}` : ' (no upstream)'}`"
            draggable="true"
            @click="git.revealCommit(row.item.oid)"
            @dblclick="git.checkout(row.item.name)"
            @contextmenu="localMenu($event, row.item.name, row.item.upstream)"
            @dragstart="drag.begin($event, { kind: 'branch', name: row.item.name, remote: false })"
            @dragend="drag.end()"
            @dragover="drag.hover($event, `branch:${row.item.name}`, ['branch', 'commit', 'stash'])"
            @dragleave="drag.leave(`branch:${row.item.name}`)"
            @drop.prevent="onDropOnBranch($event, row.item.name, false)"
          >
            <GitBranch :size="13" class="glyph" :class="{ current: row.item.is_head }" />
            <span class="name truncate">{{ row.label }}</span>
            <!-- A text arrow at this size sits so close to the digit that "↑1"
                 reads as "11", so the arrow is a glyph with a gap of its own. -->
            <span
              v-if="row.item.ahead"
              class="tick up"
              :title="`${row.item.ahead} ahead of the upstream`"
            >
              <ArrowUp :size="11" :stroke-width="2.5" />{{ row.item.ahead }}
            </span>
            <span
              v-if="row.item.behind"
              class="tick down"
              :title="`${row.item.behind} behind the upstream`"
            >
              <ArrowDown :size="11" :stroke-width="2.5" />{{ row.item.behind }}
            </span>
            <Cloud v-if="!row.item.upstream" :size="11" class="faint no-upstream" />
          </div>
        </template>
      </div>

      <!-- Remote -->
      <button class="section-title toggle" @click="open.remotes = !open.remotes">
        <ChevronRight :size="12" class="chev" :class="{ down: open.remotes }" />
        <Cloud :size="12" class="mark" />
        Remote
        <span class="count">{{ store.refs?.remotes.length ?? 0 }}</span>
      </button>
      <div v-if="open.remotes" class="group">
        <div v-for="group in remoteGroups" :key="group.remote">
          <div class="remote-name">
            <Cloud :size="11" /> {{ group.remote }}
          </div>
          <template v-for="row in shelve(group.branches, `remote:${group.remote}`)" :key="row.key">
            <button
              v-if="row.kind === 'folder'"
              class="row folder indent"
              :style="{ paddingLeft: `calc(var(--indent-2) + ${row.depth * 14}px)` }"
              @click="toggleFolder(row.path)"
            >
              <ChevronRight :size="11" class="chev" :class="{ down: !row.shut }" />
              <component :is="row.shut ? Folder : FolderOpen" :size="13" class="glyph" />
              <span class="name truncate">{{ row.label }}</span>
            </button>
            <div
              v-else
              class="row indent"
              :class="{ drop: drag.state.over === `remote:${group.remote}/${row.item.name}` }"
              :style="{ paddingLeft: `calc(var(--indent-2) + ${row.depth * 14}px)` }"
              :title="`${group.remote}/${row.item.name}`"
              draggable="true"
              @click="git.revealCommit(row.item.oid)"
              @dblclick="git.checkout(`${group.remote}/${row.item.name}`)"
              @contextmenu="remoteMenu($event, group.remote, row.item.name)"
              @dragstart="
                drag.begin($event, {
                  kind: 'branch',
                  name: `${group.remote}/${row.item.name}`,
                  remote: true
                })
              "
              @dragend="drag.end()"
            >
              <GitBranch :size="13" class="glyph remote" />
              <span class="name truncate">{{ row.label }}</span>
            </div>
          </template>
        </div>
        <p v-if="!remoteGroups.length" class="none faint">No remote branches.</p>
      </div>

      <!-- Pull requests -->
      <template v-if="forge.store.status && forge.store.status.kind !== 'none'">
        <div class="head-row">
          <button class="section-title toggle" @click="open.reviews = !open.reviews">
            <ChevronRight :size="12" class="chev" :class="{ down: open.reviews }" />
            <GitPullRequest :size="12" class="mark" />
            {{ forge.label.value }}
            <span class="count">{{ forge.store.reviews.length }}</span>
          </button>
          <button
            class="head-action"
            :disabled="!forge.usable.value"
            :title="`Open a new ${forge.label.value.toLowerCase().replace(/s$/, '')} from this branch`"
            @click="creatingReview = true"
          >
            <Plus :size="13" />
          </button>
        </div>
        <div v-if="open.reviews" class="group">
          <div
            v-for="review in forge.store.reviews"
            :key="review.number"
            class="row"
            :class="{ on: review.is_current }"
            :title="`${review.title}\n${review.source_branch} → ${review.target_branch}`"
            @dblclick="git.checkout(review.source_branch)"
            @contextmenu="
              menu.show(
                $event,
                [
                  { label: 'Open in browser', icon: ExternalLink, action: () => forge.open(review.url) },
                  {
                    label: `Check out ${review.source_branch}`,
                    icon: GitBranch,
                    action: () => git.checkout(review.source_branch)
                  },
                  { label: 'Copy link', icon: Copy, action: () => copyText(review.url, 'Link') }
                ],
                `!${review.number} ${review.title.slice(0, 40)}`
              )
            "
          >
            <GitPullRequest :size="13" class="glyph pr" />
            <span class="name truncate">
              <span class="faint">!{{ review.number }}</span>
              {{ review.title }}
            </span>
            <span v-if="review.draft" class="tick faint">draft</span>
            <button
              class="row-action"
              title="Open in the browser"
              @click.stop="forge.open(review.url)"
            >
              <ExternalLink :size="12" />
            </button>
          </div>
          <p v-if="!forge.usable.value" class="none faint">
            {{
              forge.store.status?.has_token
                ? 'No remote on this forge to read.'
                : 'Add an access token to this profile in Settings to see them.'
            }}
          </p>
          <p v-else-if="forge.store.error" class="err">{{ forge.store.error }}</p>
          <p v-else-if="!forge.store.reviews.length" class="none faint">
            {{ forge.store.loading ? 'Loading…' : 'Nothing open.' }}
          </p>
        </div>
      </template>

      <!-- Tags -->
      <button class="section-title toggle" @click="open.tags = !open.tags">
        <ChevronRight :size="12" class="chev" :class="{ down: open.tags }" />
        <Tag :size="12" class="mark" />
        Tags
        <span class="count">{{ tags.length }}</span>
      </button>
      <div v-if="open.tags" class="group">
        <div
          v-for="tag in tags"
          :key="tag.name"
          class="row"
          @contextmenu="tagMenu($event, tag.name, tag.oid)"
        >
          <Tag :size="12" class="glyph tag" />
          <span class="name truncate">{{ tag.name }}</span>
        </div>
        <p v-if="!tags.length" class="none faint">No tags.</p>
      </div>

      <!-- Stashes -->
      <button class="section-title toggle" @click="open.stashes = !open.stashes">
        <ChevronRight :size="12" class="chev" :class="{ down: open.stashes }" />
        <Archive :size="12" class="mark" />
        Stashes
        <span class="count">{{ stashes.length }}</span>
      </button>
      <div v-if="open.stashes" class="group">
        <div
          v-for="stash in stashes"
          :key="stash.index"
          class="row stash"
          :title="`${stash.files} ${stash.files === 1 ? 'file' : 'files'} · ${stash.branch ?? ''}`"
          draggable="true"
          @click="git.selectStash(stash.index)"
          @dblclick="git.stashPop(stash.index)"
          @contextmenu="stashMenu($event, stash.index, stash.message)"
          @dragstart="
            drag.begin($event, { kind: 'stash', index: stash.index, message: stash.message })
          "
          @dragend="drag.end()"
        >
          <Archive :size="12" class="glyph" />
          <span class="names">
            <span class="name truncate">{{ stash.message }}</span>
            <span class="faint meta">
              {{ stash.branch }} · {{ stash.files }}
              {{ stash.files === 1 ? 'file' : 'files' }} · {{ relativeTime(stash.time) }}
            </span>
          </span>
        </div>
        <p v-if="!stashes.length" class="none faint">Nothing stashed.</p>
      </div>
    </div>

    <PromptDialog
      v-if="prompt"
      :title="prompt.title"
      :label="prompt.label"
      :initial="prompt.initial"
      :hint="prompt.hint"
      :confirm="prompt.confirm"
      :danger="prompt.danger"
      @close="prompt = null"
      @submit="
        (value) => {
          prompt?.run(value)
          prompt = null
        }
      "
    />

    <DeleteBranchDialog v-if="deleting" :name="deleting" @close="deleting = null" />
    <ReviewDialog v-if="creatingReview" @close="creatingReview = false" />
  </aside>
</template>

<style scoped>
.side {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--bg-panel);
  border-right: 1px solid var(--line);
}

.filter {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px;
  padding: 0 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
}

.filter input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 5px 0;
  font-size: 12px;
}

.filter input:focus {
  outline: none;
}

/* Each section scrolls inside itself rather than pushing the ones below it off
   the bottom. The headings are the map of the sidebar — Local, Remote, Reviews,
   Tags, Stashes — and a repository with forty remote branches used to bury all
   of them under one list. Laid out as a column, every section keeps its heading
   in view and gives up height in proportion to how much it has; the outer
   scroll is left as the last resort, for when even the floors below do not fit.
   */
.scroll {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 14px;
}

.scroll > .toggle,
.scroll > .head-row {
  flex: none;
}

.group {
  padding-top: 2px;
  /* Each section gives up height in proportion to how much it has, so the long
     one shrinks and the short one is left alone; the floor is a single row, so
     nothing is squeezed to a sliver and nothing short is padded out. */
  flex: 0 1 auto;
  min-height: 28px;
  overflow-y: auto;
  scrollbar-width: thin;
}

/* One indent scale for the whole tree. A row's glyph sits in the same column as
   its section's icon, so a name lines up under the name of the thing it belongs
   to, and a remote's branches step in once more under the remote. */
.side {
  --pad: 8px;
  --indent: 26px;
  --indent-2: 40px;
}

/* A line above each heading rather than below it, so the rule always sits
   between two sections. Under the heading it would cut a section off from its
   own rows, and a collapsed section would leave a line under nothing. */
.toggle {
  width: 100%;
  padding-left: var(--pad);
  text-align: left;
}

.scroll > .toggle:not(:first-child) {
  margin-top: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--line-soft);
}

.toggle:hover {
  color: var(--text-dim);
}

.mark {
  flex: none;
  opacity: 0.75;
}

.chev {
  transition: transform 0.12s;
  flex: none;
}

.chev.down {
  transform: rotate(90deg);
}

.count {
  margin-left: auto;
  font-weight: 400;
}

/* The heading keeps the rule above it, so the button beside it has to sit
   inside that same box rather than after it. */
.head-row {
  display: flex;
  align-items: center;
}

.scroll > .head-row:not(:first-child) {
  margin-top: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--line-soft);
}

.head-row > .toggle {
  flex: 1;
  min-width: 0;
}

.head-action {
  flex: none;
  margin-right: 6px;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-faint);
}

.head-action:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-hover);
}

.head-action:disabled {
  opacity: 0.35;
}

/* Only on the row the pointer is over: a link icon on every review would
   compete with the titles, which are what the list is for. */
.row-action {
  flex: none;
  margin-left: 2px;
  color: var(--text-faint);
  visibility: hidden;
}

.row:hover .row-action {
  visibility: visible;
}

.row-action:hover {
  color: var(--text);
}

.row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px 3px var(--indent);
  cursor: default;
  user-select: none;
}

.row:hover {
  background: var(--bg-hover);
}

.row.on {
  background: var(--bg-active);
}

.row.on .name {
  color: #fff;
  font-weight: 600;
}

.row.drop {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
  background: rgba(79, 156, 249, 0.16);
}

/* A folder is a button, so it has to be talked out of looking like one. */
.row.folder {
  width: 100%;
  background: none;
  border: 0;
  font: inherit;
  color: inherit;
  text-align: left;
}

.row.folder:hover {
  background: var(--bg-hover);
}

.row.folder .glyph {
  color: var(--text-dim);
}

.row.stash {
  align-items: flex-start;
  padding-top: 4px;
  padding-bottom: 4px;
}

.indent {
  padding-left: var(--indent-2);
}

.glyph {
  flex: none;
  color: var(--text-faint);
}

.glyph.current {
  color: var(--accent);
}

.glyph.remote {
  color: #a58bd8;
}

.glyph.tag {
  color: var(--amber);
}

.glyph.pr {
  color: var(--green);
}

.name {
  flex: 1;
  min-width: 0;
}

.names {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.meta {
  font-size: 10px;
}

.tick {
  display: inline-flex;
  align-items: center;
  gap: 1px;
  margin-left: 3px;
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  flex: none;
}

/* Lucide draws on a padded 24-unit grid, so centring the icon box against the
   digit leaves the stroke sitting high and crowding it. Nudge the glyph rather
   than the box. */
.tick svg {
  transform: translate(-1px, 1px);
}

.up {
  color: var(--green);
}

.down {
  color: var(--accent);
}

.no-upstream {
  flex: none;
  opacity: 0.5;
}

.remote-name {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px 2px var(--indent);
  font-size: 11px;
  color: var(--text-faint);
}

.none,
.err {
  padding: 3px 12px 5px var(--indent);
  font-size: 11.5px;
  margin: 0;
}

.err {
  color: var(--red);
}
</style>

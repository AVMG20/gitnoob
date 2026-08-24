<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Archive,
  ChevronRight,
  Cloud,
  Copy,
  ExternalLink,
  GitBranch,
  GitMerge,
  GitPullRequest,
  Hash,
  Pencil,
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

const head = computed(() => store.repo?.head ?? '')
const locals = computed(() => (store.refs?.locals ?? []).filter((b) => match(b.name)))
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
            await git.cherryPick(payload.oid)
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

  // Offer the cheap answer first when there is one.
  const fastForward = await invoke<boolean>('can_fast_forward', {
    branch: target,
    onto: source
  }).catch(() => false)

  menu.show(
    event,
    [
      ...(fastForward
        ? [
            {
              label: `Fast-forward ${target} to ${source}`,
              icon: GitMerge,
              hint: 'no merge commit',
              action: async () => {
                if (target !== head.value) await git.checkout(target)
                await git.merge(source, false)
              }
            }
          ]
        : []),
      {
        label: `Merge ${source} into ${target}`,
        icon: GitMerge,
        hint: fastForward ? 'force a merge commit' : '',
        action: async () => {
          if (target !== head.value) await git.checkout(target)
          const outcome = await git.merge(source, fastForward)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: `Rebase ${target} onto ${source}`,
        icon: GitBranch,
        hint: 'rewrites history',
        danger: true,
        action: async () => {
          if (target !== head.value) await git.checkout(target)
          const outcome = await git.rebase(source)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      }
    ],
    `${source} → ${target}`
  )
}

// --- context menus

function localMenu(event: MouseEvent, name: string, upstream: string | null) {
  const isHead = name === head.value
  menu.show(
    event,
    [
      { label: 'Check out', icon: GitBranch, disabled: isHead, action: () => git.checkout(name) },
      {
        label: `Merge into ${head.value}`,
        icon: GitMerge,
        disabled: isHead,
        action: async () => {
          const outcome = await git.merge(name, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: `Rebase ${head.value} onto this`,
        icon: GitBranch,
        disabled: isHead,
        action: async () => {
          const outcome = await git.rebase(name)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      { separator: true, label: '' },
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
      {
        label: upstream ? `Push to ${upstream}` : 'Push and set upstream',
        icon: Upload,
        action: () => git.pushBranch(name, !upstream)
      },
      {
        label: 'Copy branch name',
        icon: Copy,
        action: () => copyText(name, 'Branch')
      },
      { separator: true, label: '' },
      {
        label: 'Delete branch',
        icon: Trash2,
        danger: true,
        disabled: isHead,
        action: () => git.deleteBranch(name, false)
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
        Local
        <span class="count">{{ locals.length }}</span>
      </button>
      <div v-if="open.locals">
        <div
          v-for="branch in locals"
          :key="branch.name"
          class="row"
          :class="{
            on: branch.is_head,
            drop: drag.state.over === `branch:${branch.name}`
          }"
          :title="`${branch.name}${branch.upstream ? ` → ${branch.upstream}` : ' (no upstream)'}`"
          draggable="true"
          @dblclick="git.checkout(branch.name)"
          @contextmenu="localMenu($event, branch.name, branch.upstream)"
          @dragstart="drag.begin($event, { kind: 'branch', name: branch.name, remote: false })"
          @dragend="drag.end()"
          @dragover="drag.hover($event, `branch:${branch.name}`, ['branch', 'commit', 'stash'])"
          @dragleave="drag.leave(`branch:${branch.name}`)"
          @drop.prevent="onDropOnBranch($event, branch.name, false)"
        >
          <GitBranch :size="13" class="glyph" :class="{ current: branch.is_head }" />
          <span class="name truncate">{{ branch.name }}</span>
          <span v-if="branch.ahead" class="tick up">↑{{ branch.ahead }}</span>
          <span v-if="branch.behind" class="tick down">↓{{ branch.behind }}</span>
          <Cloud v-if="!branch.upstream" :size="11" class="faint no-upstream" />
        </div>
      </div>

      <!-- Remote -->
      <button class="section-title toggle" @click="open.remotes = !open.remotes">
        <ChevronRight :size="12" class="chev" :class="{ down: open.remotes }" />
        Remote
        <span class="count">{{ store.refs?.remotes.length ?? 0 }}</span>
      </button>
      <template v-if="open.remotes">
        <div v-for="group in remoteGroups" :key="group.remote">
          <div class="remote-name">
            <Cloud :size="11" /> {{ group.remote }}
          </div>
          <div
            v-for="branch in group.branches"
            :key="branch.name"
            class="row indent"
            :class="{ drop: drag.state.over === `remote:${group.remote}/${branch.name}` }"
            draggable="true"
            @dblclick="git.checkout(`${group.remote}/${branch.name}`)"
            @contextmenu="remoteMenu($event, group.remote, branch.name)"
            @dragstart="
              drag.begin($event, {
                kind: 'branch',
                name: `${group.remote}/${branch.name}`,
                remote: true
              })
            "
            @dragend="drag.end()"
          >
            <GitBranch :size="13" class="glyph remote" />
            <span class="name truncate">{{ branch.name }}</span>
          </div>
        </div>
      </template>

      <!-- Pull requests -->
      <template v-if="forge.usable.value">
        <button class="section-title toggle" @click="open.reviews = !open.reviews">
          <ChevronRight :size="12" class="chev" :class="{ down: open.reviews }" />
          {{ forge.label.value }}
          <span class="count">{{ forge.store.reviews.length }}</span>
        </button>
        <div v-if="open.reviews">
          <div
            v-for="review in forge.store.reviews"
            :key="review.number"
            class="row"
            :class="{ on: review.is_current }"
            :title="review.title"
            @dblclick="forge.open(review.url)"
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
          </div>
          <p v-if="forge.store.error" class="err">{{ forge.store.error }}</p>
          <p v-else-if="!forge.store.reviews.length" class="none faint">
            {{ forge.store.loading ? 'Loading…' : 'Nothing open.' }}
          </p>
        </div>
      </template>

      <!-- Tags -->
      <button class="section-title toggle" @click="open.tags = !open.tags">
        <ChevronRight :size="12" class="chev" :class="{ down: open.tags }" />
        Tags
        <span class="count">{{ tags.length }}</span>
      </button>
      <div v-if="open.tags">
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
        Stashes
        <span class="count">{{ stashes.length }}</span>
      </button>
      <div v-if="open.stashes">
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

.scroll {
  overflow-y: auto;
  padding-bottom: 14px;
}

.toggle {
  width: 100%;
  text-align: left;
}

.toggle:hover {
  color: var(--text-dim);
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

.row {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 3px 10px 3px 12px;
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

.row.stash {
  align-items: flex-start;
  padding-top: 4px;
  padding-bottom: 4px;
}

.indent {
  padding-left: 22px;
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
  font-size: 10px;
  font-weight: 600;
  flex: none;
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
  gap: 5px;
  padding: 4px 10px 2px 14px;
  font-size: 11px;
  color: var(--text-faint);
}

.none,
.err {
  padding: 4px 12px 6px;
  font-size: 11.5px;
  margin: 0;
}

.err {
  color: var(--red);
}
</style>

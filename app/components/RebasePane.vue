<script setup lang="ts">
import { computed, ref } from 'vue'
import { GitBranch, RotateCcw, X } from 'lucide-vue-next'
import { relativeTime, useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import {
  ACTIONS,
  ACTION_WORDS,
  useRebase,
  type PlanRow,
  type RebaseAction
} from '~/composables/useRebase'

/**
 * The plan for an interactive rebase, in the pane the diff viewer and the
 * review page use.
 *
 * Not a dialog: a plan is worked on rather than confirmed, it wants the room,
 * and a modal scrim would put every keyboard shortcut in the window to sleep
 * for as long as it stood.
 */
const git = useGit()
const store = git.store
const menu = useContextMenu()
const rebase = useRebase()
const plan = rebase.store

/** The row being dragged, and where it would land. */
const dragging = ref<number | null>(null)
const slot = ref<number | null>(null)

/** True for a row that folds into the one above it. */
const melded = (row: PlanRow) => row.action === 'squash' || row.action === 'fixup'

const running = computed(() => !!plan.progress)

function actionMenu(event: MouseEvent, at: number) {
  menu.show(
    event,
    ACTIONS.map((action) => ({
      label: ACTION_WORDS[action].label,
      hint: `${action} — ${ACTION_WORDS[action].note}`,
      danger: action === 'drop',
      action: () => rebase.setAction(at, action)
    })),
    plan.rows[at]?.short ?? ''
  )
}

// --- dragging a row
//
// The placeholder is an index rather than a moved element: the list is drawn
// from the model, and taking the dragged row out of the DOM mid-drag is what
// cancels a drag in every browser. So the row stays where it is, faded, and a
// grey slot opens at the index the pointer is over.

function onDragStart(event: DragEvent, at: number) {
  dragging.value = at
  slot.value = at
  event.dataTransfer?.setData('text/plain', String(at))
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onDragOver(event: DragEvent, at: number) {
  if (dragging.value === null) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const above = event.clientY < box.top + box.height / 2
  slot.value = above ? at : at + 1
}

function onDrop() {
  if (dragging.value === null || slot.value === null) return finishDrag()
  // The slot counts positions in the list as it stands, so removing the row
  // first shifts everything after it down by one.
  const to = slot.value > dragging.value ? slot.value - 1 : slot.value
  rebase.move(dragging.value, to)
  finishDrag()
}

function finishDrag() {
  dragging.value = null
  slot.value = null
}

// --- the message a stopped reword is waiting for
const message = ref('')
const rewording = computed(() => plan.progress?.rewording ?? false)

/** Fills the box with what the commit says now, the first time it is shown. */
const draft = computed({
  get: () => (message.value === '' ? (plan.progress?.message ?? '') : message.value),
  set: (value: string) => {
    message.value = value
  }
})

async function saveMessage() {
  const text = draft.value.trim()
  if (!text) return
  message.value = ''
  await rebase.reword(text)
}
</script>

<template>
  <section class="rebase">
    <header class="head">
      <GitBranch :size="15" class="mark" />
      <div class="titles">
        <h2>
          <template v-if="running">
            Rebasing onto <span class="mono">{{ plan.ontoLabel }}</span>
          </template>
          <template v-else>
            Rebase {{ plan.rows.length }}
            {{ plan.rows.length === 1 ? 'commit' : 'commits' }} onto
            <span class="mono">{{ plan.ontoLabel }}</span>
          </template>
        </h2>
        <p class="sub faint">
          Oldest first — the order git replays them in.
          <template v-if="store.repo"> {{ store.repo.head }}</template>
        </p>
      </div>
      <span class="grow" />
      <button
        v-if="!running"
        class="btn icon"
        title="Put the plan back the way it was"
        :disabled="plan.loading"
        @click="rebase.reset()"
      >
        <RotateCcw :size="14" />
      </button>
      <button class="btn icon" title="Close" @click="rebase.close()">
        <X :size="16" />
      </button>
    </header>

    <div class="body">
      <div class="list-side">
        <div class="listhead">
          <span v-if="plan.loading">Reading the commits…</span>
          <span v-else>{{ plan.rows.length }} commits</span>
          <span class="grow" />
          <span v-if="!running" class="faint">Click an action to change it · drag to reorder</span>
        </div>

        <ul class="todo" @dragend="finishDrag" @drop.prevent="onDrop">
          <template v-for="(row, at) in plan.rows" :key="row.oid">
            <li v-if="slot === at && dragging !== null" class="slot" />
            <li
              class="row"
              :class="{
                melded: melded(row),
                gone: row.action === 'drop',
                ghost: dragging === at,
                here: plan.progress?.stopped === row.oid
              }"
              @dragover="onDragOver($event, at)"
            >
              <span
                class="grip"
                :draggable="!running"
                title="Drag to move"
                @dragstart="onDragStart($event, at)"
                >⠿</span
              >
              <button
                class="act"
                :class="`act-${row.action}`"
                :disabled="running"
                :title="`${row.action} — ${ACTION_WORDS[row.action].note}`"
                @click="actionMenu($event, at)"
              >
                {{ row.action }}
              </button>
              <span class="hash mono">{{ row.short }}</span>
              <span class="msg truncate" :title="row.summary">{{ row.summary }}</span>
              <span v-if="row.pushed" class="chip">on a remote</span>
              <span class="who faint">{{ row.author }}</span>
              <span class="when faint">{{ relativeTime(row.time) }}</span>
            </li>
          </template>
          <li v-if="slot === plan.rows.length && dragging !== null" class="slot" />
          <p v-if="!plan.loading && !plan.rows.length" class="none faint">
            Nothing to rebase here.
          </p>
        </ul>
      </div>

      <aside class="outcome">
        <h3>What you end up with</h3>
        <p class="sub faint">Newest first, as the graph reads.</p>
        <ul class="pv">
          <li v-for="one in [...rebase.outcome.value].reverse()" :key="one.row.oid">
            <span class="node" />
            <span class="t truncate">
              {{ one.row.summary }}
              <span v-if="one.folded || one.row.action !== 'pick'" class="tag">
                ({{
                  [
                    one.folded ? `+${one.folded} folded in` : '',
                    one.row.action === 'reword' ? 'new message' : '',
                    one.row.action === 'edit' ? 'stops here' : ''
                  ]
                    .filter(Boolean)
                    .join(', ')
                }})
              </span>
            </span>
          </li>
          <li class="base">
            <span class="node" />
            <span class="t truncate">{{ plan.ontoLabel }}</span>
          </li>
        </ul>

        <div class="tally">
          <strong>{{ plan.rows.length }}</strong> in,
          <strong>{{ rebase.outcome.value.length }}</strong> out<template v-if="rebase.dropped.value"
            > · {{ rebase.dropped.value }} dropped</template
          >
          ·
          {{
            rebase.stops.value
              ? `stops ${rebase.stops.value} ${rebase.stops.value === 1 ? 'time' : 'times'}`
              : 'runs straight through'
          }}
        </div>
      </aside>
    </div>

    <!-- Where the rebase has got to, once it is running. The same shape the
         toolbar uses for a refused push: the next step, not a dialog. -->
    <div v-if="running" class="strip">
      <span class="pill mono">{{ plan.progress?.at }} of {{ plan.progress?.total }}</span>
      <template v-if="store.status?.conflicted.length">
        <span>
          {{ store.status.conflicted.length }} conflicted
          {{ store.status.conflicted.length === 1 ? 'file' : 'files' }} in
          <span class="mono">{{ plan.progress?.summary }}</span>
        </span>
        <span class="grow" />
        <button class="btn tiny" @click="store.resolving = store.status.conflicted[0] ?? ''">
          Resolve them
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="rebase.skip()">
          Skip this commit
        </button>
      </template>

      <template v-else-if="rewording">
        <span>New message for <span class="mono">{{ plan.progress?.summary }}</span></span>
        <input
          v-model="draft"
          class="msgbox"
          type="text"
          autofocus
          @keyup.enter="saveMessage"
        />
        <button class="btn tiny" :disabled="store.busy || !draft.trim()" @click="saveMessage">
          Save and carry on
        </button>
      </template>

      <template v-else>
        <span>
          Stopped at <span class="mono">{{ plan.progress?.summary }}</span> — change what you
          like, then carry on.
        </span>
        <span class="grow" />
        <button class="btn tiny" :disabled="store.busy" @click="rebase.resume()">
          Carry on
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="rebase.skip()">
          Skip this commit
        </button>
      </template>

      <button class="btn tiny ghost" :disabled="store.busy" @click="rebase.abort()">
        Abort the rebase
      </button>
    </div>

    <footer v-else class="foot">
      <span class="cmd mono truncate">git rebase -i --autostash {{ plan.ontoLabel }}</span>
      <span class="grow" />
      <span v-if="rebase.refusal.value" class="refusal">{{ rebase.refusal.value }}</span>
      <span v-else-if="rebase.rewriting.value" class="warn-line">
        Some of these are on a remote — the push afterwards will need a force.
      </span>
      <button class="btn btn-ghost" @click="rebase.close()">Cancel</button>
      <button
        class="btn btn-primary"
        :disabled="store.busy || plan.starting || !!rebase.refusal.value || !plan.rows.length"
        @click="rebase.start()"
      >
        Start rebase
      </button>
    </footer>
  </section>
</template>

<style scoped>
.rebase {
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--bg);
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.mark {
  color: var(--accent);
  flex: none;
}

.titles h2 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}

.sub {
  margin: 0;
  font-size: 11px;
}

.grow {
  margin-left: auto;
}

.icon {
  padding: 4px 6px;
}

.body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 280px;
}

.list-side {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.listhead {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  font-size: 11px;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
}

.todo {
  flex: 1;
  min-height: 0;
  list-style: none;
  margin: 0;
  padding: 6px 8px;
  overflow-y: auto;
}

.row {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 30px;
  padding: 0 9px;
  margin-bottom: 2px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
  background: var(--bg-panel);
}

.row:hover {
  background: var(--bg-raised);
  border-color: var(--line-soft);
}

/* The row being dragged stays put, faded, so nothing jumps under the pointer. */
.row.ghost {
  opacity: 0.28;
}

/* Where it would land. */
.slot {
  height: 30px;
  margin-bottom: 2px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  border: 1px dashed var(--line);
}

.row.here {
  border-color: var(--warning-line);
  background: var(--warning-bg);
}

.grip {
  flex: none;
  width: 12px;
  text-align: center;
  color: var(--text-faint);
  cursor: grab;
  opacity: 0;
  user-select: none;
}

.row:hover .grip,
.row.ghost .grip {
  opacity: 1;
}

.grip:active {
  cursor: grabbing;
}

.act {
  flex: none;
  width: 76px;
  padding: 2px 8px;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 11px;
  font-weight: 600;
  text-align: left;
  border: 1px solid transparent;
}

.act:disabled {
  cursor: default;
}

.act-pick {
  background: var(--primary-bg);
  color: var(--accent);
  border-color: var(--primary-line);
}

.act-reword {
  background: var(--info-bg);
  color: var(--purple-soft);
  border-color: color-mix(in srgb, var(--purple) 40%, transparent);
}

.act-squash,
.act-fixup {
  background: var(--success-bg);
  color: var(--green-soft);
  border-color: var(--success-line);
}

.act-edit {
  background: var(--warning-bg);
  color: var(--amber-soft);
  border-color: var(--warning-line);
}

.act-drop {
  background: var(--danger-bg);
  color: var(--red-soft);
  border-color: var(--danger-line);
}

.hash {
  flex: none;
  font-size: 11px;
  color: var(--text-faint);
}

.msg {
  flex: 1;
  min-width: 0;
  font-size: 12px;
}

.row.gone .msg,
.row.gone .hash {
  text-decoration: line-through;
  opacity: 0.5;
}

/* A folded commit is tied to the one above it by a rail down the left. */
.row.melded {
  margin-left: 18px;
  position: relative;
}

.row.melded::before {
  content: '';
  position: absolute;
  left: -11px;
  top: -3px;
  bottom: 50%;
  width: 9px;
  border-left: 1.5px solid var(--success-line);
  border-bottom: 1.5px solid var(--success-line);
  border-bottom-left-radius: 5px;
}

.chip {
  flex: none;
  padding: 0 5px;
  border-radius: 3px;
  font-size: 10px;
  background: var(--warning-bg);
  color: var(--amber-soft);
  border: 1px solid var(--warning-line);
}

.who,
.when {
  flex: none;
  font-size: 11px;
}

.when {
  width: 82px;
  text-align: right;
}

.outcome {
  border-left: 1px solid var(--line-soft);
  padding: 12px 14px;
  overflow-y: auto;
  background: var(--bg-panel);
}

.outcome h3 {
  margin: 0 0 2px;
  font-size: 12px;
}

.pv {
  list-style: none;
  margin: 10px 0 0;
  padding: 0;
}

.pv li {
  display: flex;
  align-items: baseline;
  gap: 9px;
  padding: 4px 0;
  font-size: 12px;
}

.node {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent);
}

.pv .base .node {
  background: var(--text-faint);
}

.pv .base .t {
  color: var(--text-faint);
}

.t {
  min-width: 0;
}

.tag {
  font-family: var(--mono);
  font-size: 10px;
  color: var(--text-faint);
}

.tally {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--line-soft);
  font-size: 11px;
  color: var(--text-dim);
}

.foot,
.strip {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 14px;
  background: var(--bg-panel);
  border-top: 1px solid var(--line);
}

.cmd {
  font-size: 11px;
  color: var(--text-faint);
}

.refusal {
  font-size: 12px;
  color: var(--red);
}

.warn-line {
  font-size: 12px;
  color: var(--amber);
}

.strip {
  font-size: 12px;
  color: var(--amber-soft);
  background: var(--warning-bg);
  border-top: 1px solid var(--warning-line);
}

.pill {
  flex: none;
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 11px;
  background: color-mix(in srgb, var(--warning-line) 55%, transparent);
}

.msgbox {
  flex: 1;
  min-width: 0;
  padding: 3px 8px;
  font-size: 12px;
}

.tiny {
  font-size: 11px;
  padding: 2px 8px;
  background: var(--amber);
  color: #1a1206;
  font-weight: 600;
}

.tiny.ghost {
  background: none;
  color: var(--amber-soft);
  border: 1px solid var(--warning-line);
}

.none {
  padding: 10px 12px;
  font-size: 12px;
}
</style>

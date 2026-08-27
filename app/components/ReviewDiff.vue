<script setup lang="ts">
import { computed } from 'vue'
import ReviewThread from './ReviewThread.vue'
import CommentBox from './CommentBox.vue'
import { useReview } from '~/composables/useReview'
import type { Pending, RFileWithDiff, Thread } from '~/composables/useReview'
import type { DiffLine } from '~/composables/useGit'
import { highlightLine, languageFor } from '~/composables/useHighlight'
import { renderMarkdown } from '~/composables/useMd'

const props = defineProps<{
  file: RFileWithDiff
}>()

const review = useReview()
const store = review.store

const language = computed(() => languageFor(props.file.path))

function lineClass(origin: string) {
  if (origin === '+') return 'add'
  if (origin === '-') return 'del'
  // Git's "\ No newline at end of file": a remark about the lines around it
  // rather than a line of either file, and drawn as one.
  if (origin === '\\') return 'eof'
  return 'ctx'
}

/**
 * Highlighted lines, cached by content.
 *
 * Highlighting every line on every re-render is what made the diff view slow
 * before it cached; the cache lives as long as the language does, which is as
 * long as this file is on screen.
 */
const perLine = computed(() => {
  const cache = new Map<string, string>()
  const lang = language.value
  return (code: string) => {
    const hit = cache.get(code)
    if (hit !== undefined) return hit
    const html = highlightLine(code, lang)
    cache.set(code, html)
    return html
  }
})

/**
 * The threads standing on a line, either side of it.
 *
 * A remark anchors to the new file's numbering where it has one, and to the
 * old file's where the line it was about has since gone — which is the only
 * place an old-side thread can be drawn at all.
 */
function threadsFor(line: DiffLine): Thread[] {
  const out: Thread[] = []
  if (line.new_lineno !== null) out.push(...review.threadsAt(props.file.path, 'new', line.new_lineno))
  if (line.old_lineno !== null) out.push(...review.threadsAt(props.file.path, 'old', line.old_lineno))
  return out
}

/** The held-back remark standing on this line, when there is one. */
function pendingHere(line: DiffLine): Pending | null {
  const at = line.new_lineno ?? line.old_lineno
  if (at === null) return null
  return review.pendingAt(props.file.path, line.new_lineno !== null ? 'new' : 'old', at)
}

/** Opens a held-back remark back up in the box it was written in. */
function edit(line: DiffLine) {
  const held = pendingHere(line)
  if (!held) return
  store.drafts.lines[draftKey(line)] = held.body
  review.dropPending(held)
  review.beginDraft(held.path, held.line, held.side)
}

/** The "\\ No newline at end of file" remark, which is nobody's line to answer. */
const marker = (line: DiffLine) => line.origin === '\\'

/**
 * A line's text as HTML. The remark is git talking rather than the file, so it
 * is escaped and left alone instead of being coloured as whatever language
 * this is.
 */
const body = (line: DiffLine) =>
  marker(line) ? highlightLine(line.content, null) : perLine.value(line.content)

function draftHere(line: DiffLine) {
  const draft = store.draft
  if (!draft || draft.path !== props.file.path) return false
  return draft.side === 'new' ? draft.line === line.new_lineno : draft.line === line.old_lineno
}

function begin(line: DiffLine) {
  const at = line.new_lineno ?? line.old_lineno
  if (at === null) return
  review.beginDraft(props.file.path, at, line.new_lineno !== null ? 'new' : 'old')
}

/** The store keeps the open reply box as a number; this is its toggle. */
function toggleReply(id: number) {
  store.replyingTo = store.replyingTo === id ? null : id
}

function locate(thread: Thread) {
  if (!thread.root.path) return
  store.tab = 'files'
  store.selectedPath = thread.root.path
}

/** Where the kept text of this line's half-written remark lives. */
function draftKey(line: DiffLine) {
  const at = line.new_lineno ?? line.old_lineno
  return at === null ? '' : review.lineDraftKey(props.file.path, line.new_lineno !== null ? 'new' : 'old', at)
}
</script>

<template>
  <section class="file" data-testid="review-diff" :data-file="props.file.path">

    <div class="diff">
      <p v-if="props.file.binary || !props.file.hunks.length" class="note dim">
        No text diff for this file.
      </p>

      <template v-else>
        <div v-for="(hunk, index) in props.file.hunks" :key="index" class="hunk">
          <div class="hunk-head mono truncate">{{ hunk.header }}</div>
          <template
            v-for="(line, at) in hunk.lines"
            :key="`${at}:${line.origin}${line.old_lineno}-${line.new_lineno}`"
          >
            <div
              class="diff-line"
              :class="lineClass(line.origin)"
              :data-line="marker(line) ? null : (line.new_lineno ?? line.old_lineno ?? '')"
              :data-side="marker(line) ? null : line.new_lineno !== null ? 'new' : 'old'"
            >
              <span class="no">{{ line.old_lineno ?? '' }}</span>
              <span class="no">{{ line.new_lineno ?? '' }}</span>
              <span class="sign">{{ line.origin === ' ' ? '' : line.origin }}</span>
              <span class="text" v-html="body(line)" />
              <button
                v-if="!marker(line)"
                class="line-add"
                title="Comment on this line"
                @click="begin(line)"
              >
                +
              </button>
            </div>

            <div v-if="draftHere(line)" class="line-extra">
              <CommentBox
                v-model="store.drafts.lines[draftKey(line)]"
                compact
                autofocus
                :busy="store.sending"
                send-label="Add to review"
                second-label="Comment now"
                placeholder="Comment on this line…"
                @update:model-value="review.saveDrafts()"
                @send="review.queueDraft"
                @second="review.sendDraft"
                @cancel="review.cancelDraft"
              />
            </div>

            <!-- Written, and waiting for the verdict that will carry it. -->
            <div v-if="pendingHere(line)" class="line-extra">
              <div class="pending" data-testid="pending-remark">
                <div class="pending-head">
                  <span class="chip">Pending</span>
                  <span class="faint">goes out with your review</span>
                  <span class="grow" />
                  <button class="quiet" title="Edit this remark" @click="edit(line)">Edit</button>
                  <button
                    class="quiet"
                    data-testid="pending-drop"
                    title="Take this remark back"
                    @click="review.dropPending(pendingHere(line)!)"
                  >
                    Discard
                  </button>
                </div>
                <div class="md-body" v-html="renderMarkdown(pendingHere(line)!.body)" />
              </div>
            </div>

            <div v-for="thread in threadsFor(line)" :key="thread.key" class="line-extra">
              <ReviewThread
                :thread="thread"
                :busy="store.sending"
                :reply-to="store.replyingTo"
                @reply="review.reply"
                @toggle-reply="toggleReply"
                @locate="locate"
                @resolve="review.resolveThread"
              />
            </div>
          </template>
        </div>
      </template>
    </div>

</section>
</template>

<style scoped>
.file {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.note {
  margin: 0;
  padding: 10px 12px;
}

/* Only the diff scrolls sideways, so the head and the threads under a line
   stay put however long a line runs. */
.diff {
  overflow-x: auto;
}

.hunk {
  min-width: 0;
}

.hunk-head {
  position: sticky;
  /* Flush with the top of the patch. The offset here used to clear the file's
     own head; that head is the page's file bar now, and since `.diff` scrolls
     sideways it is also this header's scroll port — so any offset was drawn as
     a gap above the header and the header itself over the first two lines. */
  top: 0;
  z-index: 2;
  padding: 3px 10px;
  color: var(--text-faint);
  background: var(--bg-raised);
  border-top: 1px solid var(--line-soft);
  border-bottom: 1px solid var(--line-soft);
  font-size: 11px;
}

.diff-line {
  display: grid;
  grid-template-columns: 42px 42px 14px minmax(0, 1fr) 22px;
  align-items: center;
  width: max-content;
  min-width: 100%;
  height: 18px;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 18px;
  white-space: pre;
}

.diff-line .no {
  padding-right: 8px;
  text-align: right;
  color: var(--text-faint);
  user-select: none;
}

.diff-line .sign {
  text-align: center;
  user-select: none;
}

.diff-line .text {
  padding-right: 10px;
  tab-size: 4;
}

.diff-line.add {
  background: var(--success-bg);
}

.diff-line.add .sign {
  color: var(--green-soft);
}

.diff-line.del {
  background: var(--danger-bg);
}

.diff-line.eof,
.diff-line.eof .sign {
  color: var(--text-faint);
  font-style: italic;
}

.diff-line.del .sign {
  color: var(--red-soft);
}

/* The review's whole gesture: a round accent chip that surfaces on the line
   under the pointer. It borrows the app's own accent rather than drawing a
   boxed form control on every row. */
.line-add {
  justify-self: end;
  align-self: center;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  margin-right: 4px;
  border-radius: 50%;
  background: var(--accent);
  color: var(--on-accent);
  font-size: 13px;
  font-weight: 500;
  line-height: 1;
  box-shadow: 0 1px 4px var(--shadow);
  opacity: 0;
  transform: scale(0.8);
  transition: opacity 0.1s, transform 0.1s;
}

.diff-line:hover .line-add {
  opacity: 1;
  transform: scale(1);
}

.line-add:hover {
  background: var(--accent-hover);
}

/* What stands under a line — the threads already there, and the one being
   written — indented to sit under the code column rather than under the
   numbers, and pinned to it when the patch is scrolled sideways. */
.pending {
  border-left: 2px dashed var(--amber);
  padding: 2px 0 2px 10px;
}

.pending-head {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 11px;
  color: var(--text-faint);
}

.pending-head .grow {
  flex: 1;
}

.pending .chip {
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--amber) 45%, transparent);
  color: var(--amber-soft);
  font-size: 10px;
  font-weight: 600;
}

.pending .quiet {
  padding: 1px 6px;
  border-radius: 4px;
  color: var(--text-faint);
  font-size: 11px;
}

.pending .quiet:hover {
  color: var(--text);
  background: var(--bg-hover);
}

/* A remark not yet sent, a size down from a comment on the page. */
.pending .md-body {
  font-size: 12px;
}

.pending .md-body :deep(p) {
  margin: 4px 0;
}

.line-extra {
  position: sticky;
  left: 0;
  margin-left: 98px;
  margin-right: 12px;
  max-width: min(760px, calc(100% - 110px));
  padding: 6px 12px;
  border-top: 1px solid var(--line-soft);
  background: var(--bg-deep);
}
</style>

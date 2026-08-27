/**
 * Git's failures, said in a sentence a person can act on.
 *
 * Git explains itself to someone who already knows git: "Your local changes to
 * the following files would be overwritten by checkout" is four lines of
 * plumbing around the one thing that matters, which is that the switch did not
 * happen and the way out is to commit or stash. Each rule here recognises one
 * such refusal and answers it with what to do about it.
 *
 * A sentence is what went wrong and what to do about it, in that order and in
 * as few words as it takes. It is read in a corner of the window by somebody
 * who was doing something else, so it says "Cannot switch branch. Commit or
 * stash your changes first." rather than naming the branch, counting the files
 * and quoting the tool — all of which are a click away.
 *
 * Nothing is thrown away. The whole message git wrote is kept as the detail
 * behind the sentence, because the rules cover the failures worth naming and
 * not the ones nobody has hit yet — and for those the raw text is the only
 * thing that helps.
 */

export interface Explained {
  /** One line, in the imperative where there is something to do. */
  title: string
  /** Everything git said, or `null` when the title already is that. */
  detail: string | null
  /** True for the failures the window itself already answers. */
  quiet: boolean
}

interface Rule {
  when: RegExp
  say: string
  /**
   * Set where the app's own response says it better than a notice would.
   *
   * A merge that stops on conflicts opens the resolver with every conflicted
   * file in it — a notice repeating that, over the page that is already
   * answering it, is one more thing to dismiss.
   */
  quiet?: boolean
}

/**
 * Ordered: the first match wins, so the specific refusals come before the
 * general ones they would otherwise be swallowed by.
 */
const RULES: Rule[] = [
  {
    when: /untracked working tree files would be overwritten/i,
    say: 'New files are in the way. Move or delete them first.'
  },
  {
    when: /would be overwritten by checkout|Cannot switch to .*would be overwritten|Please commit your changes or stash them/i,
    say: 'Cannot switch branch. Commit or stash your changes first.'
  },
  {
    when: /would be overwritten by (?:merge|rebase)/i,
    say: 'Not with open changes. Commit or stash them first.'
  },
  {
    when: /cannot (?:pull|rebase) with rebase: You have unstaged changes|[Cc]annot rebase: You have unstaged changes/i,
    say: 'Rebase needs a clean tree. Commit or stash first.'
  },
  {
    when: /Automatic merge failed|^CONFLICT \(|\nCONFLICT \(/i,
    say: 'It stopped on conflicts. Resolve the files, then commit.',
    quiet: true
  },
  {
    when: /You have not concluded your merge|MERGE_HEAD exists|a rebase is in progress|is already in progress/i,
    say: 'Something here is half finished. Finish or abort it first.'
  },
  {
    when: /Updates were rejected|non-fast-forward|fetch first|behind its remote counterpart/i,
    say: 'The remote is ahead. Pull first, then push.'
  },
  {
    when: /has no upstream branch|no upstream configured/i,
    say: 'Not on the remote yet. Push to create it there.'
  },
  {
    when: /Need to specify how to reconcile divergent branches|You have divergent branches/i,
    say: 'Both sides moved. Pick merge or rebase for pulls.'
  },
  {
    when: /refusing to merge unrelated histories/i,
    say: 'No shared history. Git will not merge these.'
  },
  {
    when: /is not fully merged/i,
    say: 'It has commits no other branch has. Delete by force to lose them.'
  },
  {
    when: /Permission denied \(publickey\)/i,
    say: 'The remote refused your SSH key. Check the profile.'
  },
  {
    when: /could not read Username|Authentication failed|Invalid username or password|Support for password authentication was removed|401 Unauthorized/i,
    say: 'The remote refused your credentials. Check the profile.'
  },
  {
    when: /403 Forbidden|remote: Write access to repository not granted/i,
    say: 'Read access only. Your account cannot write here.'
  },
  {
    when: /Could not resolve host|Could not resolve hostname|Failed to connect|Connection timed out|Network is unreachable/i,
    say: 'Cannot reach the remote. Check your connection.'
  },
  {
    when: /Repository not found|does not appear to be a git repository/i,
    say: 'The remote has no such repository.'
  },
  {
    when: /index\.lock|Unable to create .*\.lock|cannot lock ref/i,
    say: 'Another git process is running. Wait, or delete index.lock.'
  },
  {
    when: /did not match any file\(s\) known to git|unknown revision or path not in the working tree|not something we can merge|bad revision/i,
    say: 'Git does not know that branch, commit or path.'
  },
  {
    when: /nothing to commit|no changes added to commit/i,
    say: 'Nothing staged to commit.'
  },
  {
    when: /Your branch is up to date|Already up to date/i,
    say: 'Already up to date.'
  },
  {
    when: /would clobber existing tag|already exists/i,
    say: 'Something by that name is already there.'
  }
]

/** The first line worth showing, for a message no rule recognises. */
function firstLine(text: string): string {
  const line = text
    .split('\n')
    .map((one) => one.trim())
    .find((one) => one.length > 0)
  return line ?? text.trim()
}

/**
 * Turns a failure into a sentence and the evidence behind it.
 *
 * `text` is whatever was going to be written to the log — usually the label of
 * what was being done, a colon, and git's own words. Both halves are matched
 * against, because the interesting part can be in either.
 */
export function explain(text: string): Explained {
  const whole = text.trim()
  if (!whole) return { title: 'Something went wrong', detail: null, quiet: false }

  const rule = RULES.find((one) => one.when.test(whole))
  if (rule) return { title: rule.say, detail: whole, quiet: !!rule.quiet }

  const head = firstLine(whole)
  // A one-line message is its own explanation; repeating it under itself as a
  // detail would only give the toast a disclosure triangle that reveals what is
  // already on screen.
  return { title: head, detail: whole === head ? null : whole, quiet: false }
}

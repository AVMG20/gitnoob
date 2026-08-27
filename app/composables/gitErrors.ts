/**
 * Git's failures, said in a sentence a person can act on.
 *
 * Git explains itself to someone who already knows git: "Your local changes to
 * the following files would be overwritten by checkout" is four lines of
 * plumbing around the one thing that matters, which is that the switch did not
 * happen and the way out is to commit or stash. Each rule here recognises one
 * such refusal and answers it with what to do about it.
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
    say: 'Untracked files are in the way. Move, delete, or commit them first.'
  },
  {
    when: /would be overwritten by checkout|would be overwritten by (?:merge|rebase)|Please commit your changes or stash them/i,
    say: 'You have open changes that this would overwrite. Commit or stash them first.'
  },
  {
    when: /cannot (?:pull|rebase) with rebase: You have unstaged changes|cannot rebase: You have unstaged changes|Cannot rebase: You have unstaged changes/i,
    say: 'Rebasing needs a clean working tree. Commit or stash your changes first.'
  },
  {
    when: /Automatic merge failed|^CONFLICT \(|\nCONFLICT \(/i,
    say: 'It stopped on conflicts. Resolve the conflicted files, then commit.',
    quiet: true
  },
  {
    when: /You have not concluded your merge|MERGE_HEAD exists|a rebase is in progress|is already in progress/i,
    say: 'Something is still half-done in this repository. Finish or abort it first.'
  },
  {
    when: /Updates were rejected|non-fast-forward|fetch first|behind its remote counterpart/i,
    say: 'The remote has commits you have not got. Pull first, then push again.'
  },
  {
    when: /has no upstream branch|no upstream configured/i,
    say: 'This branch is not on the remote yet. Push it to create it there.'
  },
  {
    when: /Need to specify how to reconcile divergent branches|You have divergent branches/i,
    say: 'The branch and its remote have both moved. Choose merge or rebase for the pull.'
  },
  {
    when: /refusing to merge unrelated histories/i,
    say: 'These two branches share no history, so git will not merge them.'
  },
  {
    when: /is not fully merged/i,
    say: 'That branch has commits no other branch has. Delete it by force to lose them.'
  },
  {
    when: /Permission denied \(publickey\)/i,
    say: 'The remote refused your SSH key. Check which key this profile pins.'
  },
  {
    when: /could not read Username|Authentication failed|Invalid username or password|Support for password authentication was removed|401 Unauthorized/i,
    say: 'The remote would not take your credentials. Check the token for this profile.'
  },
  {
    when: /403 Forbidden|remote: Write access to repository not granted/i,
    say: 'Your account may read this repository but not write to it.'
  },
  {
    when: /Could not resolve host|Could not resolve hostname|Failed to connect|Connection timed out|Network is unreachable/i,
    say: 'The remote could not be reached. Check the connection and the remote address.'
  },
  {
    when: /Repository not found|does not appear to be a git repository/i,
    say: 'The remote does not have that repository, or will not admit it to you.'
  },
  {
    when: /index\.lock|Unable to create .*\.lock|cannot lock ref/i,
    say: 'Another git process has this repository locked. Wait for it, or remove the lock file.'
  },
  {
    when: /did not match any file\(s\) known to git|unknown revision or path not in the working tree|not something we can merge|bad revision/i,
    say: 'Git does not know that branch, commit or path.'
  },
  {
    when: /nothing to commit|no changes added to commit/i,
    say: 'There is nothing staged to commit.'
  },
  {
    when: /Your branch is up to date|Already up to date/i,
    say: 'Nothing to do — this is already up to date.'
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

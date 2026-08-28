/**
 * The line typed at the log's prompt, split the way a shell would split it.
 *
 * Quotes group, so `commit -m "fix the thing"` is three arguments; a
 * backslash keeps the next character, so a quote can be part of a word. A
 * leading `git` is dropped: people type what they would type in a terminal,
 * and the prompt already says which program this is.
 */
export function parseCommandLine(line: string): { args: string[] } | { error: string } {
  const args: string[] = []
  let word = ''
  let inWord = false
  let quote: '"' | "'" | null = null

  for (let i = 0; i < line.length; i++) {
    const ch = line[i]!
    if (quote) {
      if (ch === quote) quote = null
      else if (ch === '\\' && quote === '"' && i + 1 < line.length) word += line[++i]
      else word += ch
    } else if (ch === '"' || ch === "'") {
      quote = ch
      inWord = true
    } else if (ch === '\\' && i + 1 < line.length) {
      word += line[++i]
      inWord = true
    } else if (ch === ' ' || ch === '\t') {
      if (inWord) args.push(word)
      word = ''
      inWord = false
    } else {
      word += ch
      inWord = true
    }
  }
  if (quote) return { error: `Missing a closing ${quote}` }
  if (inWord) args.push(word)

  if (args[0] === 'git') args.shift()
  if (!args.length) return { error: 'Nothing to run' }
  return { args }
}

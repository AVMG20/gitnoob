/**
 * Runs before every test file.
 *
 * happy-dom fires `selectionchange` the moment a selection moves, inside the
 * call that moved it. Browsers queue it as a task. CodeMirror sets the DOM
 * selection in the middle of an update and expects to hear about it later,
 * so with happy-dom's timing it can hear about it in the middle of that
 * update, try to update again and throw "Calls to EditorView.update are not
 * allowed while an update is in progress". Whether it does depends on how
 * fast the machine is, which is why it showed up on CI and not locally.
 * Queuing the event as a browser does takes the race away.
 */
if (typeof document !== 'undefined') {
  const dispatch = document.dispatchEvent.bind(document)
  let queued = false
  document.dispatchEvent = (event: Event) => {
    if (event.type !== 'selectionchange') return dispatch(event)
    // happy-dom fires one for every collapse, moved or not; a browser fires
    // one, later, for however many moves happened before it got round to it.
    if (queued) return true
    queued = true
    setTimeout(() => {
      queued = false
      dispatch(event)
    }, 0)
    return true
  }
}

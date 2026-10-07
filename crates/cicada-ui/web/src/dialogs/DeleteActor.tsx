// Delete Actor confirmation: DELETE /actors/{path} removes the state and purges the mailbox, so the user types the
// URI to confirm.

import { useState } from 'react';
import { ApiError } from '../api/client';
import { useConsole } from '../app/context';
import { count } from '../lib/format';
import { Button, Icon, ProblemCallout } from '../halo';
import { Dialog } from '../ui/Dialog';

/** [onDeleted] runs after the actor is gone (also when it already was). */
export function DeleteActorDialog({ uri, version, pending, morePending, onClose, onDeleted }: { uri: string; version: number; pending: number; morePending: boolean; onClose: () => void; onDeleted: () => void }) {
  const ctx = useConsole();
  const [typed, setTyped] = useState('');
  const [deleting, setDeleting] = useState(false);
  const [problem, setProblem] = useState<unknown>(null);

  async function remove() {
    if (deleting || typed !== uri) return;
    setDeleting(true);
    setProblem(null);
    try {
      await ctx.api.deleteActor(uri);
      onClose();
      onDeleted();
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        onClose();
        onDeleted();
        return;
      }
      setProblem(e);
    } finally {
      setDeleting(false);
    }
  }

  return (
    <Dialog labelledBy="delete-title" onClose={onClose}>
      <div className="cc-dialog-body cc-dialog-body-top">
        <div className="cc-row cc-row-top">
          <span className="cc-danger-mark"><Icon name="trash" size={18} /></span>
          <div>
            <h2 id="delete-title" className="cc-dialog-title">Delete Actor</h2>
            <div className="cc-muted cc-small">
              {`This removes its state (version ${version}) and purges its `}
              <b className="cc-ink">{(morePending ? 'more than ' : '') + count(pending, 'pending message')}</b>
              . It can’t be undone.
            </div>
          </div>
        </div>
        <div className="cc-field">
          <label className="cc-label" htmlFor="delete-confirm">Type <span className="mono cc-code">{uri}</span> to confirm</label>
          <input id="delete-confirm" className="cc-input mono" placeholder={uri} autoComplete="off" spellCheck={false} value={typed}
            onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void remove(); }} />
          {typed !== '' && typed !== uri ? <span className="cc-hint">Doesn’t match yet.</span> : null}
        </div>
        <div className="cc-result">{problem ? <ProblemCallout error={problem} retry={() => void remove()} /> : null}</div>
      </div>
      <footer className="cc-dialog-foot">
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <button type="button" className="ha-btn cc-btn-danger" disabled={typed !== uri || deleting} onClick={() => void remove()}>Delete Actor</button>
      </footer>
    </Dialog>
  );
}

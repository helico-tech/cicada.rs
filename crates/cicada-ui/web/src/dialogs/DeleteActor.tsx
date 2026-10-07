// Delete Actor confirmation: DELETE /actors/{path} removes the state and purges the mailbox, so the user types the
// URI to confirm.

import { useState } from 'react';
import { ApiError } from '../api/client';
import { useConsole } from '../app/context';
import { count } from '../lib/format';
import { Button, Field, ProblemCallout } from '../sx';
import { Dialog, DialogBody, DialogFoot, DialogHeader, DialogResult } from '../ui/Dialog';

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
      <DialogHeader
        id="delete-title" title="Delete actor" icon={{ name: 'trash', tone: 'danger' }} onClose={onClose}
        caption={<>
          {`This removes its state (version ${version}) and purges its `}
          <b className="cc-ink">{(morePending ? 'more than ' : '') + count(pending, 'pending message')}</b>
          . It can’t be undone.
        </>}
      />
      <DialogBody>
        <Field id="delete-confirm" label={<>Type <span className="mono cc-code">{uri}</span> to confirm</>}
          hint={typed !== '' && typed !== uri ? 'Doesn’t match yet.' : undefined}>
          <input id="delete-confirm" className="sx-input sx-mono" placeholder={uri} autoComplete="off" spellCheck={false} value={typed}
            onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void remove(); }} />
        </Field>
      </DialogBody>
      <DialogResult>{problem ? <ProblemCallout error={problem} retry={() => void remove()} /> : null}</DialogResult>
      <DialogFoot>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="danger" icon="trash" disabled={typed !== uri || deleting} onClick={() => void remove()}>Delete actor</Button>
      </DialogFoot>
    </Dialog>
  );
}

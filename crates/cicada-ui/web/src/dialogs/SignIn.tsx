// Sign-in for a bearer-token API: asked for on a 401, kept in this tab's session storage.

import { useState } from 'react';
import { Button } from '../halo';
import { Dialog, DialogHeader } from '../ui/Dialog';

export function SignInDialog({ onClose, onSubmit }: { onClose: () => void; onSubmit: (token: string) => void }) {
  const [value, setValue] = useState('');
  const submit = () => onSubmit(value.trim());
  return (
    <Dialog labelledBy="signin-title" onClose={onClose}>
      <DialogHeader id="signin-title" title="Sign In" caption="This node’s API answered 401 Unauthorized." onClose={onClose} />
      <div className="cc-dialog-body">
        <div className="cc-field">
          <label className="cc-label" htmlFor="token">API token</label>
          <input
            id="token" className="cc-input mono" type="password" autoComplete="off" data-autofocus value={value}
            onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') submit(); }}
          />
          <span className="cc-hint">Sent as a bearer token to this node only, and kept in this tab until it closes.</span>
        </div>
      </div>
      <footer className="cc-dialog-foot">
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <Button variant="primary" icon="key" onClick={submit}>Sign In</Button>
      </footer>
    </Dialog>
  );
}

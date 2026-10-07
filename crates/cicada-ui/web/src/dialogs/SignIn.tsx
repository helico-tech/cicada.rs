// Sign-in for a bearer-token API: asked for on a 401, kept in this tab's session storage.

import { useState } from 'react';
import { Button, Field } from '../sx';
import { Dialog, DialogBody, DialogFoot, DialogHeader } from '../ui/Dialog';

export function SignInDialog({ onClose, onSubmit }: { onClose: () => void; onSubmit: (token: string) => void }) {
  const [value, setValue] = useState('');
  const submit = () => onSubmit(value.trim());
  return (
    <Dialog size="s" labelledBy="signin-title" onClose={onClose}>
      <DialogHeader id="signin-title" title="Sign in" caption="This node’s API answered 401 Unauthorized." onClose={onClose} />
      <DialogBody>
        <Field id="token" label="API token" hint="Sent as a bearer token to this node only, and kept in this tab until it closes.">
          <input
            id="token" className="sx-input sx-mono" type="password" autoComplete="off" data-autofocus value={value}
            onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') submit(); }}
          />
        </Field>
      </DialogBody>
      <DialogFoot>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" icon="key" onClick={submit}>Sign in</Button>
      </DialogFoot>
    </Dialog>
  );
}

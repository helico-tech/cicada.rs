// Entry point of index.html: reads config.json (where the API is) and starts the console.

import { createRoot } from 'react-dom/client';
import './styles/sensus/tokens.css';
import './styles/sensus/bundle.css';
import './styles/app.css';
import { App } from './app/App';
import { Callout } from './sx';

async function start(): Promise<void> {
  const root = createRoot(document.getElementById('app')!);
  try {
    const response = await fetch('config.json', { cache: 'no-store' });
    if (!response.ok) throw new Error(`config.json answered ${response.status}`);
    const config = (await response.json()) as { apiPath?: unknown };
    if (typeof config.apiPath !== 'string') throw new Error('config.json has no apiPath');
    root.render(<App apiPath={config.apiPath} />);
  } catch (e) {
    root.render(
      <div className="cc-boot">
        <Callout kind="user" title="The console cannot start" detail={`Its configuration did not load (${e instanceof Error ? e.message : String(e)}).`} />
      </div>,
    );
  }
}

void start();

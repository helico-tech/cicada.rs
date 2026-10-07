// Light and dark mode: Halo's `halo-light` / `halo-dark` classes on the root element. The console follows the system
// setting until the viewer picks one with the top bar's switch; that choice is remembered in this browser.

import { useEffect, useState } from 'react';
import { local } from './storage';

const KEY = 'cicada-console-theme';

function systemDark(): boolean {
  return matchMedia('(prefers-color-scheme: dark)').matches;
}

function apply(dark: boolean): void {
  document.documentElement.classList.toggle('halo-dark', dark);
  document.documentElement.classList.toggle('halo-light', !dark);
}

/** The current theme and a toggle; applies it to the document. */
export function useTheme(): { dark: boolean; toggle: () => void } {
  const [chosen, setChosen] = useState<string | null>(() => local.get(KEY)); // 'light', 'dark', or null: the system's
  const [system, setSystem] = useState(systemDark);
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const listener = () => setSystem(media.matches);
    media.addEventListener('change', listener);
    return () => media.removeEventListener('change', listener);
  }, []);
  const dark = chosen ? chosen === 'dark' : system;
  useEffect(() => apply(dark), [dark]);
  return {
    dark,
    toggle() {
      const next = dark ? 'light' : 'dark';
      local.set(KEY, next);
      setChosen(next);
    },
  };
}

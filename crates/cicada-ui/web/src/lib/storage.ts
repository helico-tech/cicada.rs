// Browser storage that may be missing: private windows and blocked storage just forget.

interface Store {
  get(key: string): string | null;
  set(key: string, value: string | null): void;
}

function storage(name: 'sessionStorage' | 'localStorage'): Store {
  return {
    get(key) {
      try {
        return globalThis[name].getItem(key);
      } catch {
        return null;
      }
    },
    set(key, value) {
      try {
        if (value === null) globalThis[name].removeItem(key);
        else globalThis[name].setItem(key, value);
      } catch {
        // Not remembered; the console still works.
      }
    },
  };
}

/** This tab only (the API token). */
export const session = storage('sessionStorage');

/** This browser (preferences such as the theme). */
export const local = storage('localStorage');

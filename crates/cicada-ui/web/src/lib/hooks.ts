// Small React hooks shared by the pages.

import { useEffect, useState } from 'react';

/** [value], updated only after it stopped changing for [ms]. */
export function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

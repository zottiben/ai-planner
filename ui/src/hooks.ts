// Loading data, without a data library.
//
// Three states and a refetch is the whole requirement. `useResource` keeps the previous
// value visible while the next one loads, which is what makes a refresh - and, in PR5,
// a live update - not blink the board out of existence on every change.

import { useCallback, useEffect, useRef, useState } from "react";

export interface Resource<T> {
  data: T | undefined;
  error: Error | undefined;
  /** True only for the first load. A background refresh must not blank the screen. */
  loading: boolean;
  reload: () => void;
}

/**
 * @param deps Re-fetch when any of these change. Pass a liveness generation here to
 *   follow out-of-process writes - the previous value stays on screen while the next
 *   one loads, so a live update does not blink the board out of existence.
 */
export function useResource<T>(
  fetcher: () => Promise<T>,
  deps: readonly unknown[],
): Resource<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);

  // A slow request that lands after a newer one would otherwise overwrite fresher
  // data with staler data. The generation counter drops anything that is not current.
  const generation = useRef(0);

  useEffect(() => {
    const mine = ++generation.current;
    let cancelled = false;

    fetcher()
      .then((value) => {
        if (cancelled || mine !== generation.current) return;
        setData(value);
        setError(undefined);
      })
      .catch((err: unknown) => {
        if (cancelled || mine !== generation.current) return;
        setError(err instanceof Error ? err : new Error(String(err)));
      })
      .finally(() => {
        if (!cancelled && mine === generation.current) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { data, error, loading, reload };
}

/** Persisted UI preferences - which columns are collapsed, which repos are open. */
export function useStored<T>(key: string, initial: T): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });

  const update = useCallback(
    (next: T) => {
      setValue(next);
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // A full or disabled storage is not a reason to stop working.
      }
    },
    [key],
  );

  return [value, update];
}

// Following the database while four agents write to it.
//
// A stale board is a worse lie than no board, so this is not a nicety. The server
// announces a change (D4) and the client refetches; the event carries no payload
// because "something changed" is all a refetch needs to know, and inventing a
// granular protocol would be a second model of the schema to keep honest.

import { useEffect, useRef, useState } from "react";

import { token } from "./api";

export type Connection = "connecting" | "live" | "reconnecting";

interface Live {
  /** Bumped on every announced change. Use it as a dependency to revalidate. */
  generation: number;
  connection: Connection;
}

export function useLive(): Live {
  const [generation, setGeneration] = useState(0);
  const [connection, setConnection] = useState<Connection>("connecting");
  // Distinguishes the first connect from a reconnect, so the indicator can be honest
  // about which one is happening.
  const opened = useRef(false);

  useEffect(() => {
    // EventSource cannot set headers, which is why the token has a query form.
    const source = new EventSource(`/api/events?t=${encodeURIComponent(token)}`);

    source.onopen = () => {
      // A reconnect means we were disconnected, and anything could have happened while
      // we were not listening - so reopening is itself a reason to refetch.
      if (opened.current) setGeneration((n) => n + 1);
      opened.current = true;
      setConnection("live");
    };

    source.addEventListener("changed", () => setGeneration((n) => n + 1));

    // EventSource retries on its own, so this reports rather than repairs. Claiming
    // "live" over a dead stream is the one thing this must never do.
    source.onerror = () => setConnection(opened.current ? "reconnecting" : "connecting");

    return () => source.close();
  }, []);

  return { generation, connection };
}

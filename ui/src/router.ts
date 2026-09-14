// A router, in about sixty lines.
//
// The whole app has three routes. Pulling in a router library to serve them would add
// more to the committed bundle than the feature is worth (D2), and the History API
// plus `useSyncExternalStore` is the same thing without the dependency.

import { useCallback, useSyncExternalStore } from "react";

const listeners = new Set<() => void>();

function announce() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener("popstate", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("popstate", listener);
  };
}

export function navigate(path: string, options: { replace?: boolean } = {}) {
  if (path === window.location.pathname) return;
  window.history[options.replace ? "replaceState" : "pushState"](null, "", path);
  announce();
}

export function usePath(): string {
  return useSyncExternalStore(
    subscribe,
    () => window.location.pathname,
    () => "/",
  );
}

export function useNavigate() {
  return useCallback(navigate, []);
}

export interface Route {
  /** No plan selected - the sidebar is the whole screen. */
  kind: "home" | "plan";
  planSlug?: string;
  sliceKey?: string;
  tab?: "board" | "plan";
}

/**
 * `/`, `/plan/:slug`, `/plan/:slug/about`, `/plan/:slug/slice/:key`.
 *
 * Slug rather than id, so a URL survives a database rebuilt from imports - ids are
 * assigned by insertion order and a slug is what a person recognises.
 */
export function parse(path: string): Route {
  const parts = path.split("/").filter(Boolean);
  if (parts[0] !== "plan" || !parts[1]) return { kind: "home" };

  const route: Route = { kind: "plan", planSlug: decodeURIComponent(parts[1]), tab: "board" };
  if (parts[2] === "slice" && parts[3]) {
    route.sliceKey = decodeURIComponent(parts[3]);
  } else if (parts[2] === "about") {
    route.tab = "plan";
  }
  return route;
}

export function planPath(slug: string) {
  return `/plan/${encodeURIComponent(slug)}`;
}

export function slicePath(slug: string, key: string) {
  return `${planPath(slug)}/slice/${encodeURIComponent(key)}`;
}

export function aboutPath(slug: string) {
  return `${planPath(slug)}/about`;
}

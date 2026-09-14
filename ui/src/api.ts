// Talking to the board server.
//
// The token arrives in the URL because that is the only channel a freshly opened tab
// has. It is moved into memory and stripped from the address bar immediately: leaving
// it there puts a live credential into the history, into any screenshot, and into every
// Referer header the page sends.

import type {
  Board,
  BoardColumn,
  LogEntry,
  Meta,
  Plan,
  PlanBundle,
  PlanSummary,
  RepoSummary,
  Slice,
  SliceDetail,
  Status,
} from "./types";

const TOKEN_KEY = "ai-planner.token";

function takeToken(): string {
  const url = new URL(window.location.href);
  const fromUrl = url.searchParams.get("t");
  if (fromUrl) {
    // Session storage, not local: the token dies with the server that minted it, so
    // keeping it beyond the tab would only ever produce a confusing 401 later.
    sessionStorage.setItem(TOKEN_KEY, fromUrl);
    url.searchParams.delete("t");
    window.history.replaceState(null, "", url.pathname + url.search + url.hash);
    return fromUrl;
  }
  return sessionStorage.getItem(TOKEN_KEY) ?? "";
}

export const token = takeToken();

/** A refusal from the server, with the structured detail it sent. */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly holder?: string;
  readonly worktree?: string;
  readonly slice?: string;

  constructor(
    status: number,
    body: {
      error?: string;
      code?: string;
      holder?: string;
      worktree?: string;
      slice?: string;
    },
  ) {
    super(body.error ?? `request failed (${status})`);
    this.name = "ApiError";
    this.status = status;
    this.code = body.code ?? "unknown";
    this.holder = body.holder;
    this.worktree = body.worktree;
    this.slice = body.slice;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, {
    ...init,
    headers: {
      "x-planner-token": token,
      ...(init?.body ? { "content-type": "application/json" } : {}),
      ...init?.headers,
    },
  });

  if (!response.ok) {
    // A non-JSON error body means something upstream of the handler broke; say that
    // rather than throwing a JSON parse error over the top of it.
    const body = await response.json().catch(() => ({
      error: `${response.status} ${response.statusText}`,
    }));
    throw new ApiError(response.status, body);
  }
  return response.status === 204 ? (undefined as T) : ((await response.json()) as T);
}

export const api = {
  meta: () => request<Meta>("/meta"),
  repos: () => request<RepoSummary[]>("/repos"),
  repoBoard: (id: number) => request<BoardColumn[]>(`/repos/${id}/board`),
  plans: (params: { repo?: number; incomplete?: boolean; q?: string } = {}) => {
    const query = new URLSearchParams();
    if (params.repo !== undefined) query.set("repo", String(params.repo));
    if (params.incomplete) query.set("incomplete", "true");
    if (params.q) query.set("q", params.q);
    const suffix = query.toString();
    return request<PlanSummary[]>(`/plans${suffix ? `?${suffix}` : ""}`);
  },
  plan: (id: number) => request<PlanBundle>(`/plans/${id}`),
  board: (id: number) => request<Board>(`/plans/${id}/board`),
  planLog: (id: number, limit?: number) =>
    request<LogEntry[]>(`/plans/${id}/log${limit ? `?limit=${limit}` : ""}`),
  slice: (id: number) => request<SliceDetail>(`/slices/${id}`),

  setSliceStatus: (id: number, status: Status, reason?: string) =>
    post<Slice>(`/slices/${id}/status`, { status, reason }),
  claim: (id: number, worktree?: string) => post<Slice>(`/slices/${id}/claim`, { worktree }),
  release: (id: number) => post<Slice>(`/slices/${id}/release`, {}),
  editSlice: (id: number, patch: Partial<Slice>) =>
    request<Slice>(`/slices/${id}`, { method: "PATCH", body: JSON.stringify(patch) }),
  addNote: (planId: number, body: string, slice?: string, kind?: string) =>
    post<{ id: number }>(`/plans/${planId}/log`, { body, slice, kind }),
  setPlanStatus: (id: number, status: Status) => post<Plan>(`/plans/${id}/status`, { status }),
  answerQuestion: (id: number, answer: string) =>
    post<{ ok: boolean }>(`/questions/${id}/answer`, { answer }),
};

function post<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, { method: "POST", body: JSON.stringify(body) });
}

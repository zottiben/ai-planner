// The guards in front of the one write that cannot be taken back.
//
// The server enforces both of these as well, and has its own tests for that. These are
// here because a dialog whose Delete button is live before the question is answered is
// a dialog that will eventually be clicked through.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "./api";
import { DeletePlanDialog, DeleteSliceDialog } from "./Delete";
import { Toaster } from "./Toast";
import type { PlanRemoval, PlanSummary, Slice, SliceRemoval } from "./types";

const plan: PlanSummary = {
  id: 7,
  repo_id: 1,
  repo_name: "widget",
  slug: "ship-the-widget",
  title: "Ship the widget",
  status: "active",
  summary: null,
  ticket_key: null,
  ticket_url: null,
  base_branch: null,
  owner: null,
  source_path: null,
  rev: 1,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  slices: 3,
  done: 1,
  percent: 33,
  open_questions: 0,
  last_activity: null,
};

const slice: Slice = {
  id: 42,
  plan_id: 7,
  ord: 10,
  key: "PR2",
  title: "Build the thing",
  status: "active",
  scope_md: "",
  demo_md: null,
  estimate_files: null,
  branch: null,
  base_branch: null,
  pr_url: null,
  worktree_path: null,
  claimed_by: null,
  claimed_at: null,
  blocked_reason: null,
  started_at: null,
  completed_at: null,
  rev: 1,
  updated_at: "2026-01-01T00:00:00Z",
};

const planRemoval: PlanRemoval = {
  plan_id: 7,
  slug: "ship-the-widget",
  title: "Ship the widget",
  repo: "widget",
  status: "active",
  sections: 4,
  slices: 3,
  decisions: 2,
  questions: 0,
  gotchas: 0,
  log_entries: 17,
  handoffs: 1,
  sources: 0,
  imports: 0,
  embeddings: 0,
  held: [],
  imported_from: [],
};

const sliceRemoval: SliceRemoval = {
  slice_id: 42,
  plan_id: 7,
  plan_slug: "ship-the-widget",
  key: "PR2",
  title: "Build the thing",
  status: "active",
  branch: null,
  pr_url: null,
  detached_log_entries: 5,
  detached_questions: 0,
  dependents: 0,
  embeddings: 0,
  held: null,
};

function show(node: React.ReactNode) {
  return render(<Toaster>{node}</Toaster>);
}

function deleteButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Delete" }) as HTMLButtonElement;
}

// Explicit, because vitest is not running with `globals`, so Testing Library never
// registers its own afterEach - and one dialog left on screen makes the next test's
// query find two of everything.
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("DeletePlanDialog", () => {
  it("stays disabled until the slug is typed back exactly", async () => {
    vi.spyOn(api, "planRemoval").mockResolvedValue(planRemoval);
    const remove = vi.spyOn(api, "deletePlan").mockResolvedValue(planRemoval);
    const onDeleted = vi.fn();

    show(<DeletePlanDialog plan={plan} onDeleted={onDeleted} onCancel={vi.fn()} />);

    // What is about to go, counted by the server rather than guessed at here.
    await screen.findByText("3 slices");
    await screen.findByText("17 notes");

    const field = screen.getByLabelText("Type ship-the-widget to confirm");
    expect(deleteButton().disabled).toBe(true);

    fireEvent.change(field, { target: { value: "ship-the-widgets" } });
    expect(deleteButton().disabled).toBe(true);

    fireEvent.change(field, { target: { value: "ship-the-widget" } });
    expect(deleteButton().disabled).toBe(false);

    fireEvent.click(deleteButton());
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
    expect(remove).toHaveBeenCalledWith(7, "ship-the-widget", false);
  });

  it("will not delete work somebody is holding without being told twice", async () => {
    vi.spyOn(api, "planRemoval").mockResolvedValue({
      ...planRemoval,
      held: [{ key: "PR2", claimed_by: "agent-a", worktree_path: "/tmp/widget-a" }],
    });
    const remove = vi.spyOn(api, "deletePlan").mockResolvedValue(planRemoval);

    show(<DeletePlanDialog plan={plan} onDeleted={vi.fn()} onCancel={vi.fn()} />);

    // Whose work it is, and where - not merely that something is in the way.
    await screen.findByText(/agent-a/);
    expect(screen.getByText(/\/tmp\/widget-a/)).toBeTruthy();

    const field = screen.getByLabelText("Type ship-the-widget to confirm");
    fireEvent.change(field, { target: { value: "ship-the-widget" } });
    expect(deleteButton().disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox"));
    expect(deleteButton().disabled).toBe(false);

    fireEvent.click(deleteButton());
    await waitFor(() => expect(remove).toHaveBeenCalledWith(7, "ship-the-widget", true));
  });
});

describe("DeleteSliceDialog", () => {
  it("says what survives, and asks for no typing", async () => {
    vi.spyOn(api, "sliceRemoval").mockResolvedValue(sliceRemoval);
    const remove = vi.spyOn(api, "deleteSlice").mockResolvedValue(sliceRemoval);
    const onDeleted = vi.fn();

    show(<DeleteSliceDialog slice={slice} onDeleted={onDeleted} onCancel={vi.fn()} />);

    await screen.findByText(/5 notes written against it stay on the plan/);
    expect(screen.queryByRole("textbox")).toBeNull();

    await waitFor(() => expect(deleteButton().disabled).toBe(false));
    fireEvent.click(deleteButton());
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
    expect(remove).toHaveBeenCalledWith(42, false);
  });

  it("keeps the dialog open when the server refuses", async () => {
    vi.spyOn(api, "sliceRemoval").mockResolvedValue(sliceRemoval);
    vi.spyOn(api, "deleteSlice").mockRejectedValue(new Error("slice PR2 is claimed"));
    const onDeleted = vi.fn();

    show(<DeleteSliceDialog slice={slice} onDeleted={onDeleted} onCancel={vi.fn()} />);

    await waitFor(() => expect(deleteButton().disabled).toBe(false));
    fireEvent.click(deleteButton());

    // The reason reaches the screen, and nothing pretends the slice went.
    await screen.findByText("slice PR2 is claimed");
    expect(onDeleted).not.toHaveBeenCalled();
    expect(deleteButton().disabled).toBe(false);
  });
});

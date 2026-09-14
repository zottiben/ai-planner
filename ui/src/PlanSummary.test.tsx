import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PlanSummary } from "./PlanSummary";

describe("PlanSummary", () => {
  it("expands the clamped description in place and collapses it again", () => {
    const text = "A long plan description ".repeat(20);
    render(<PlanSummary text={text} />);

    const summary = screen.getByRole("button");
    expect(summary.textContent).toBe(text);
    expect(summary.getAttribute("aria-expanded")).toBe("false");
    expect(summary.classList.contains("is-expanded")).toBe(false);

    fireEvent.click(summary);
    expect(summary.getAttribute("aria-expanded")).toBe("true");
    expect(summary.classList.contains("is-expanded")).toBe(true);

    // It is a real button, so Enter/Space activation comes from the browser rather
    // than a hand-rolled key listener. A second activation restores the compact view.
    fireEvent.click(summary);
    expect(summary.getAttribute("aria-expanded")).toBe("false");
    expect(summary.classList.contains("is-expanded")).toBe(false);
  });
});

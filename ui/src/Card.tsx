// One slice, as a card.
//
// The footer answers the question a board is opened to answer: is anyone on this, and
// where has it got to. A claim wins the space over a branch, because a claim is the one
// fact that says "someone is working on this right now".

import { prLabel, shortPath, statusColour } from "./format";
import { Branch, Person, PullRequest } from "./icons";
import type { Slice } from "./types";

interface Props {
  slice: Slice;
  current: boolean;
  onOpen: (slice: Slice) => void;
}

export function Card({ slice, current, onOpen }: Props) {
  const claim = slice.claimed_by;
  return (
    <button
      className={`card${current ? " is-current" : ""}`}
      style={{ borderLeftColor: statusColour(slice.status) }}
      onClick={() => onOpen(slice)}
      aria-current={current ? "true" : undefined}
    >
      <span className="card-top">
        <span className="card-key">{slice.key}</span>
        <span className="card-title">{slice.title}</span>
      </span>

      <span className="card-foot">
        {claim ? (
          <Chip
            className="claim"
            icon={<Person size={10} />}
            label={claim}
            title={
              slice.worktree_path ? `${claim} in ${slice.worktree_path}` : `claimed by ${claim}`
            }
          />
        ) : (
          slice.branch && (
            <Chip
              className="branch"
              icon={<Branch size={10} />}
              label={slice.branch}
              title={slice.branch}
            />
          )
        )}

        {claim && slice.worktree_path && (
          <Chip
            className="branch"
            label={shortPath(slice.worktree_path)}
            title={slice.worktree_path}
          />
        )}

        {slice.pr_url && (
          <Chip
            className="pr spacer"
            icon={<PullRequest size={10} />}
            label={prLabel(slice.pr_url)}
            title={slice.pr_url}
          />
        )}

        {!slice.pr_url && slice.estimate_files !== null && (
          <Chip
            className="spacer"
            label={`~${slice.estimate_files}f`}
            title={`about ${slice.estimate_files} files`}
          />
        )}
      </span>
    </button>
  );
}

/** The label lives in its own element because `text-overflow` does not apply to a
 *  bare text node inside a flex container - it would be cut off square instead. */
function Chip({
  className,
  icon,
  label,
  title,
}: {
  className: string;
  icon?: React.ReactNode;
  label: string;
  title: string;
}) {
  return (
    <span className={`chip ${className}`} title={title}>
      {icon}
      <span>{label}</span>
    </span>
  );
}

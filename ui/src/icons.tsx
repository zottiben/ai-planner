// Inline SVG, sized in em so icons scale with the text they sit beside.
//
// An icon font or an icon package would both weigh more than these six paths, and the
// bundle ships inside the binary (D2).

interface IconProps {
  size?: number;
  className?: string;
}

function Svg({
  size = 12,
  className,
  children,
}: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function Chevron(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6 3.5 10.5 8 6 12.5" />
    </Svg>
  );
}

export function Branch(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="4.5" cy="3.5" r="1.75" />
      <circle cx="4.5" cy="12.5" r="1.75" />
      <circle cx="11.5" cy="5.5" r="1.75" />
      <path d="M4.5 5.25v5.5M11.5 7.25c0 2-1.8 3.1-3.5 3.4" />
    </Svg>
  );
}

export function PullRequest(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="4" cy="3.5" r="1.75" />
      <circle cx="4" cy="12.5" r="1.75" />
      <circle cx="12" cy="12.5" r="1.75" />
      <path d="M4 5.25v5.5M12 10.75V6.5a2 2 0 0 0-2-2H7.5M9.5 2.5 7 4.5l2.5 2" />
    </Svg>
  );
}

export function Person(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="5" r="2.5" />
      <path d="M2.75 13.5a5.25 5.25 0 0 1 10.5 0" />
    </Svg>
  );
}

export function Collapse(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M9.5 3.5 5 8l4.5 4.5" />
    </Svg>
  );
}

export function Expand(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6.5 3.5 11 8l-4.5 4.5" />
    </Svg>
  );
}

/** The mark. Three columns of a board, the middle one mid-flight. */
export function Logo({ size = 16 }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden="true">
      <rect x="1" y="2" width="3.6" height="12" rx="1.2" fill="currentColor" opacity="0.32" />
      <rect x="6.2" y="2" width="3.6" height="8.4" rx="1.2" fill="currentColor" opacity="0.62" />
      <rect x="11.4" y="2" width="3.6" height="5" rx="1.2" fill="currentColor" />
    </svg>
  );
}

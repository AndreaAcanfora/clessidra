type P = { className?: string };

export const Hourglass = ({ className }: P) => (
  <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
    <rect x="5" y="2.5" width="14" height="2.2" rx="1.1" fill="currentColor" />
    <rect x="5" y="19.3" width="14" height="2.2" rx="1.1" fill="currentColor" />
    <path d="M6.6 4.7h10.8c0 3.8-3.6 5.4-4.4 7.3.8 1.9 4.4 3.5 4.4 7.3H6.6c0-3.8 3.6-5.4 4.4-7.3-.8-1.9-4.4-3.5-4.4-7.3Z" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    <path className="sand" d="M8.4 7.4h7.2c-.7 1.6-2.6 2.6-3.6 3.5-1-.9-2.9-1.9-3.6-3.5Z" />
    <path className="sand" d="M7.6 18.8c1.2-1.7 2.9-2.5 4.4-2.5s3.2.8 4.4 2.5Z" />
  </svg>
);

const stroke = { fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export const PlusIcon = () => (
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M8 3v10M3 8h10" {...stroke} />
  </svg>
);
export const PauseIcon = () => (
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M5.5 3.5v9M10.5 3.5v9" {...stroke} />
  </svg>
);
export const PlayIcon = () => (
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M5 3.5l7 4.5-7 4.5z" {...stroke} />
  </svg>
);
export const GearIcon = () => (
  <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
    <circle cx="8" cy="8" r="2.2" {...stroke} />
    <path d="M8 1.8v1.6M8 12.6v1.6M14.2 8h-1.6M3.4 8H1.8M12.4 3.6l-1.1 1.1M4.7 11.3l-1.1 1.1M12.4 12.4l-1.1-1.1M4.7 4.7 3.6 3.6" {...stroke} />
  </svg>
);
export const FinderIcon = () => (
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M2 4.5h4l1.2 1.5H14v6.5H2z" {...stroke} />
  </svg>
);
export const CloseIcon = () => (
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path d="M4 4l8 8M12 4l-8 8" {...stroke} />
  </svg>
);

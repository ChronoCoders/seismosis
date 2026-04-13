'use client';

import Link from 'next/link';
import type { DisplayEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude, formatRelativeTime, formatDepth } from '@/lib/magnitude';

interface EarthquakeListProps {
  events: DisplayEvent[];
}

export function EarthquakeList({ events }: EarthquakeListProps) {
  if (events.length === 0) {
    return (
      <div className="flex items-center justify-center h-32 text-text-muted text-sm">
        Veri bekleniyor…
      </div>
    );
  }

  return (
    <ul className="divide-y divide-border">
      {events.map((event) => {
        const info = getMagnitudeInfo(event.magnitude);
        return (
          <li key={event.source_id}>
            <Link
              href={`/deprem/${encodeURIComponent(event.source_id)}`}
              className="flex items-center gap-3 px-4 py-2.5 hover:bg-surface-elevated transition-colors"
            >
              {/* Magnitude */}
              <div
                className={`w-12 shrink-0 text-center rounded py-0.5 border text-sm font-bold font-mono ${info.bgClass} ${info.borderClass} ${info.textClass}`}
              >
                {formatMagnitude(event.magnitude)}
              </div>

              {/* Location + time */}
              <div className="flex-1 min-w-0">
                <p className="text-sm text-text-primary truncate leading-tight">
                  {event.region_name ?? event.source_id}
                </p>
                <div className="flex items-center gap-2 mt-0.5">
                  <span className="text-xs text-text-muted font-mono">
                    {formatRelativeTime(event.event_time)}
                  </span>
                  <span className="text-text-muted text-xs">&middot;</span>
                  <span className="text-xs text-text-muted">
                    {formatDepth(event.depth_km)}
                  </span>
                  {event.is_live && (
                    <>
                      <span className="text-text-muted text-xs">&middot;</span>
                      <span className="text-xs text-accent">CANLI</span>
                    </>
                  )}
                </div>
              </div>

              {/* Arrow */}
              <svg
                width="14"
                height="14"
                viewBox="0 0 14 14"
                fill="none"
                className="text-text-muted shrink-0"
              >
                <path
                  d="M5 3l4 4-4 4"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

'use client';

import Link from 'next/link';
import type { DisplayEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude, formatRelativeTime, formatDepth } from '@/lib/magnitude';

interface EarthquakeListProps {
  events: DisplayEvent[];
}

// ── Alert level badge ─────────────────────────────────────────────────────────

const ALERT_COLORS: Record<string, string> = {
  RED:    'bg-red-950 text-mag-red border-mag-red/70',
  ORANGE: 'bg-orange-950 text-mag-orange border-mag-orange/70',
  YELLOW: 'bg-yellow-950 text-mag-yellow border-mag-yellow/70',
};

function AlertBadge({ level }: { level: string }) {
  return (
    <span className={`text-[10px] font-bold px-1.5 rounded border tracking-widest ${ALERT_COLORS[level] ?? 'border-border text-text-muted'}`}>
      {level}
    </span>
  );
}

// ── Aftershock / mainshock badge ──────────────────────────────────────────────

function SeismicTypeBadge({
  isAfterShock,
  mainshockMagnitude,
}: {
  isAfterShock: boolean;
  mainshockMagnitude?: number | null;
}) {
  if (isAfterShock) {
    return (
      <span className="text-[10px] font-semibold px-1.5 rounded border border-border bg-surface-elevated text-text-secondary tracking-wide">
        ARTÇI{mainshockMagnitude != null ? ` M${formatMagnitude(mainshockMagnitude)}` : ''}
      </span>
    );
  }
  return (
    <span className="text-[10px] font-semibold px-1.5 rounded border border-border bg-surface-elevated text-text-muted tracking-wide">
      ANA ŞOK
    </span>
  );
}

// ── List ──────────────────────────────────────────────────────────────────────

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
        const info = getMagnitudeInfo(event.ml_magnitude ?? event.magnitude);
        const showMlMag =
          event.ml_magnitude !== undefined &&
          Math.abs(event.ml_magnitude - event.magnitude) >= 0.1;

        return (
          <li key={event.source_id}>
            <Link
              href={`/deprem/${encodeURIComponent(event.source_id)}`}
              className="flex items-start gap-3 px-4 py-2.5 hover:bg-surface-elevated transition-colors"
            >
              {/* Magnitude column */}
              <div className="flex flex-col items-center shrink-0 gap-0.5 pt-0.5">
                <div
                  className={`w-12 text-center rounded py-0.5 border text-sm font-bold font-mono ${info.bgClass} ${info.borderClass} ${info.textClass}`}
                >
                  {formatMagnitude(event.ml_magnitude ?? event.magnitude)}
                </div>
                {showMlMag && (
                  <span className="text-[10px] font-mono text-text-muted leading-none">
                    ham {formatMagnitude(event.magnitude)}
                  </span>
                )}
              </div>

              {/* Content */}
              <div className="flex-1 min-w-0">
                <p className="text-sm text-text-primary truncate leading-tight">
                  {event.region_name ?? event.source_id}
                </p>
                <div className="flex items-center flex-wrap gap-1.5 mt-1">
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
                      <span className="text-[10px] font-semibold text-accent tracking-widest">CANLI</span>
                    </>
                  )}
                  {event.alert_level && (
                    <AlertBadge level={event.alert_level} />
                  )}
                  {event.is_live && event.is_aftershock !== undefined && (
                    <SeismicTypeBadge
                      isAfterShock={event.is_aftershock}
                      mainshockMagnitude={event.mainshock_magnitude}
                    />
                  )}
                </div>
              </div>

              {/* Arrow */}
              <svg
                width="14"
                height="14"
                viewBox="0 0 14 14"
                fill="none"
                className="text-text-muted shrink-0 mt-1"
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

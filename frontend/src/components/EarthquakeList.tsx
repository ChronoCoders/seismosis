'use client';

import Link from 'next/link';
import type { DisplayEvent } from '@/types';
import {
  getMagnitudeInfo,
  formatMagnitude,
  formatRelativeTime,
  formatDepth,
} from '@/lib/magnitude';

interface EarthquakeListProps {
  events: DisplayEvent[];
}

// ── Source network badge ──────────────────────────────────────────────────────

const SOURCE_STYLES: Record<string, string> = {
  USGS: 'bg-blue-950/60 text-blue-400 border-blue-800/50',
  EMSC: 'bg-purple-950/60 text-purple-400 border-purple-800/50',
  AFAD: 'bg-teal-950/60 text-teal-400 border-teal-800/50',
};

function SourceBadge({ network }: { network: string }) {
  const cls =
    SOURCE_STYLES[network.toUpperCase()] ??
    'bg-surface-elevated text-text-muted border-border';
  return (
    <span
      className={`shrink-0 text-[9px] font-bold px-1.5 py-px rounded border tracking-widest ${cls}`}
    >
      {network.toUpperCase()}
    </span>
  );
}

// ── Alert level badge ─────────────────────────────────────────────────────────

const ALERT_STYLES: Record<string, string> = {
  RED:    'text-mag-red border-mag-red/60',
  ORANGE: 'text-mag-orange border-mag-orange/60',
  YELLOW: 'text-mag-yellow border-mag-yellow/60',
};

// ── List ──────────────────────────────────────────────────────────────────────

export function EarthquakeList({ events }: EarthquakeListProps) {
  if (events.length === 0) {
    return (
      <div className="flex items-center justify-center h-32 text-text-muted text-xs">
        Veri bekleniyor…
      </div>
    );
  }

  return (
    <ul>
      {events.map((event) => {
        const displayMag = event.ml_magnitude ?? event.magnitude;
        const info = getMagnitudeInfo(displayMag);
        const showMlDiff =
          event.ml_magnitude !== undefined &&
          Math.abs(event.ml_magnitude - event.magnitude) >= 0.1;

        return (
          <li key={event.source_id} className="border-b border-border/50 last:border-0">
            <Link
              href={`/deprem/${encodeURIComponent(event.source_id)}`}
              className="flex items-start gap-2.5 px-3 py-2.5 hover:bg-surface-elevated transition-colors"
            >
              {/* Magnitude badge */}
              <div
                className={`shrink-0 w-[50px] rounded py-1 text-center border ${info.bgClass} ${info.borderClass}`}
              >
                <div
                  className={`text-[15px] font-bold font-mono leading-none ${info.textClass}`}
                >
                  {formatMagnitude(displayMag)}
                </div>
                <div
                  className={`text-[8px] uppercase tracking-wider mt-0.5 leading-none ${info.textClass} opacity-75`}
                >
                  {info.label}
                </div>
              </div>

              {/* Main content */}
              <div className="flex-1 min-w-0">
                {/* Row 1: location + source */}
                <div className="flex items-start justify-between gap-1 mb-1">
                  <p
                    className="text-xs text-text-primary leading-tight truncate flex-1"
                    title={event.region_name ?? event.source_id}
                  >
                    {event.region_name ?? event.source_id}
                  </p>
                  <SourceBadge network={event.source_network} />
                </div>

                {/* Row 2: metadata chips */}
                <div className="flex items-center flex-wrap gap-x-1.5 gap-y-0.5">
                  <span className="text-[10px] font-mono text-text-muted">
                    {formatRelativeTime(event.event_time)}
                  </span>
                  <span className="text-border text-[10px]">·</span>
                  <span className="text-[10px] text-text-muted">
                    {formatDepth(event.depth_km)}
                  </span>

                  {event.is_live && (
                    <>
                      <span className="text-border text-[10px]">·</span>
                      <span className="text-[9px] font-bold text-accent tracking-widest">
                        CANLI
                      </span>
                    </>
                  )}

                  {event.alert_level && (
                    <span
                      className={`text-[9px] font-bold px-1 rounded border tracking-widest ${
                        ALERT_STYLES[event.alert_level] ?? 'text-text-muted border-border'
                      }`}
                    >
                      {event.alert_level}
                    </span>
                  )}

                  {event.is_live && event.is_aftershock !== undefined && (
                    <span className="text-[9px] font-semibold text-text-muted tracking-wide">
                      {event.is_aftershock
                        ? `ARTÇI${event.mainshock_magnitude != null ? ` M${formatMagnitude(event.mainshock_magnitude)}` : ''}`
                        : 'ANA ŞOK'}
                    </span>
                  )}

                  {showMlDiff && (
                    <span className="text-[9px] font-mono text-text-muted">
                      ham {formatMagnitude(event.magnitude)}
                    </span>
                  )}
                </div>
              </div>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

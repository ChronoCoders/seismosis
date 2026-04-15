'use client';

import { useMemo } from 'react';
import Link from 'next/link';
import type { DisplayEvent } from '@/types';
import type { WsStatus } from '@/hooks/useWebSocket';
import {
  getMagnitudeInfo,
  formatMagnitude,
  formatRelativeTime,
  formatDepth,
} from '@/lib/magnitude';

interface Props {
  events: DisplayEvent[];
  wsStatus: WsStatus;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const HIST_BANDS = [
  { key: 'minor',    label: 'M < 2',  midMag: 0.1, maxMag: 2   },
  { key: 'light',    label: 'M 2–4',  midMag: 2.1, maxMag: 4   },
  { key: 'moderate', label: 'M 4–6',  midMag: 4.1, maxMag: 6   },
  { key: 'strong',   label: 'M 6–8',  midMag: 6.1, maxMag: 8   },
  { key: 'major',    label: 'M 8+',   midMag: 8.1, maxMag: 999 },
];

function getBandIdx(mag: number): number {
  const idx = HIST_BANDS.findIndex((b) => mag < b.maxMag);
  return idx === -1 ? HIST_BANDS.length - 1 : idx;
}

function isReviewed(q: string) {
  return q === 'A' || q === 'B';
}

const SOURCE_STYLES: Record<string, string> = {
  USGS: 'bg-blue-950/60 text-blue-400 border-blue-800/50',
  EMSC: 'bg-purple-950/60 text-purple-400 border-purple-800/50',
  AFAD: 'bg-teal-950/60 text-teal-400 border-teal-800/50',
};

const WS_LABELS: Record<WsStatus, { label: string; dot: string; text: string }> = {
  connected:    { label: 'CANLI',        dot: 'bg-mag-green animate-pulse', text: 'text-mag-green'  },
  connecting:   { label: 'BAĞLANIYOR',   dot: 'bg-mag-yellow',              text: 'text-mag-yellow' },
  disconnected: { label: 'BAĞLANTI YOK', dot: 'bg-text-muted',              text: 'text-text-muted' },
  error:        { label: 'HATA',         dot: 'bg-mag-red',                  text: 'text-mag-red'   },
};

// ── Quality distribution histogram ────────────────────────────────────────────

function QualityHistogram({ events }: { events: DisplayEvent[] }) {
  const bandData = useMemo(() => {
    const counts = HIST_BANDS.map(() => ({ reviewed: 0, automatic: 0 }));
    for (const e of events) {
      const idx = getBandIdx(e.ml_magnitude ?? e.magnitude);
      if (isReviewed(e.quality_indicator)) counts[idx].reviewed++;
      else counts[idx].automatic++;
    }
    return HIST_BANDS.map((band, i) => ({
      ...band,
      ...counts[i],
      total: counts[i].reviewed + counts[i].automatic,
    }));
  }, [events]);

  const maxTotal = Math.max(...bandData.map((b) => b.total), 1);

  return (
    <div>
      <p className="text-[9px] font-semibold uppercase tracking-widest text-text-muted mb-4">
        Büyüklük × Kalite Dağılımı
      </p>
      <div className="space-y-4">
        {bandData.map((band) => {
          const info = getMagnitudeInfo(band.midMag);
          const pctR = (band.reviewed / maxTotal) * 100;
          const pctA = (band.automatic / maxTotal) * 100;
          return (
            <div key={band.key}>
              <div className="flex items-center justify-between mb-1.5">
                <span className={`text-xs font-mono font-semibold ${info.textClass}`}>
                  {band.label}
                </span>
                <div className="flex items-center gap-1.5 text-[10px] font-mono">
                  <span className="text-mag-green">{band.reviewed}</span>
                  <span className="text-border">+</span>
                  <span className="text-text-muted">{band.automatic}</span>
                  <span className="text-border ml-1">= {band.total}</span>
                </div>
              </div>
              <div className="h-5 bg-surface-elevated rounded overflow-hidden flex gap-px">
                <div
                  className="h-full transition-all duration-500"
                  style={{ width: `${pctR}%`, backgroundColor: info.dotColor, opacity: 0.9 }}
                  title={`${band.reviewed} gözden geçirilmiş`}
                />
                <div
                  className="h-full transition-all duration-500"
                  style={{ width: `${pctA}%`, backgroundColor: info.dotColor, opacity: 0.3 }}
                  title={`${band.automatic} otomatik`}
                />
              </div>
            </div>
          );
        })}
      </div>
      {/* Legend */}
      <div className="flex items-center gap-5 mt-5 pt-4 border-t border-border/40">
        <div className="flex items-center gap-1.5">
          <div className="w-3 h-3 rounded-sm bg-mag-green" style={{ opacity: 0.9 }} />
          <span className="text-[10px] text-text-muted">Gözden geçirilmiş (A/B)</span>
        </div>
        <div className="flex items-center gap-1.5">
          <div className="w-3 h-3 rounded-sm bg-text-muted" style={{ opacity: 0.4 }} />
          <span className="text-[10px] text-text-muted">Otomatik (C/D)</span>
        </div>
      </div>

      {/* Summary counts */}
      {events.length > 0 && (
        <div className="mt-5 pt-4 border-t border-border/40 grid grid-cols-2 gap-3">
          <div className="bg-surface-elevated rounded border border-border px-3 py-2 text-center">
            <div className="text-base font-bold font-mono text-mag-green">
              {bandData.reduce((s, b) => s + b.reviewed, 0)}
            </div>
            <div className="text-[9px] text-text-muted mt-0.5">Gözden geçirilmiş</div>
          </div>
          <div className="bg-surface-elevated rounded border border-border px-3 py-2 text-center">
            <div className="text-base font-bold font-mono text-text-secondary">
              {bandData.reduce((s, b) => s + b.automatic, 0)}
            </div>
            <div className="text-[9px] text-text-muted mt-0.5">Otomatik</div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Realtime tab ──────────────────────────────────────────────────────────────

export default function RealtimeTab({ events, wsStatus }: Props) {
  const wsCfg = WS_LABELS[wsStatus];
  const liveCount = events.filter((e) => e.is_live).length;
  const displayEvents = events.slice(0, 100);

  return (
    <div className="flex flex-1 min-h-0 overflow-hidden">

      {/* ── Left: scrolling event stream ── */}
      <div className="flex flex-col flex-[3] min-h-0 border-r border-border">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-2.5 border-b border-border shrink-0 bg-surface">
          <div className="flex items-center gap-2">
            <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${wsCfg.dot}`} />
            <h2 className="text-[9px] font-semibold uppercase tracking-widest text-text-muted">
              Olay Akışı
            </h2>
            <span className={`text-[9px] font-bold tracking-widest ${wsCfg.text}`}>
              {wsCfg.label}
            </span>
          </div>
          <div className="flex items-center gap-3 text-[9px] font-mono text-text-muted">
            <span className="text-accent">{liveCount} canlı</span>
            <span>{displayEvents.length} gösteriliyor</span>
          </div>
        </div>

        {/* Stream list */}
        <div className="overflow-y-auto flex-1 overscroll-contain">
          {displayEvents.length === 0 ? (
            <div className="flex items-center justify-center h-32 text-text-muted text-sm">
              Olay bekleniyor…
            </div>
          ) : (
            <ul>
              {displayEvents.map((event) => {
                const displayMag = event.ml_magnitude ?? event.magnitude;
                const info = getMagnitudeInfo(displayMag);
                const srcCls =
                  SOURCE_STYLES[event.source_network.toUpperCase()] ??
                  'bg-surface-elevated text-text-muted border-border';

                return (
                  <li
                    key={event.source_id}
                    className={`border-b border-border/40 last:border-0 ${
                      event.is_live ? 'bg-accent/[0.03]' : ''
                    }`}
                  >
                    <Link
                      href={`/deprem/${encodeURIComponent(event.source_id)}`}
                      className="flex items-center gap-3 px-4 py-2 hover:bg-surface-elevated transition-colors"
                    >
                      {/* Magnitude */}
                      <div
                        className={`shrink-0 w-12 text-center rounded py-1 border ${info.bgClass} ${info.borderClass}`}
                      >
                        <div
                          className={`text-sm font-bold font-mono leading-none ${info.textClass}`}
                        >
                          {formatMagnitude(displayMag)}
                        </div>
                      </div>

                      {/* Content */}
                      <div className="flex-1 min-w-0">
                        <p
                          className="text-xs text-text-primary truncate"
                          title={event.region_name ?? event.source_id}
                        >
                          {event.region_name ?? event.source_id}
                        </p>
                        <div className="flex items-center gap-1.5 mt-0.5 flex-wrap">
                          <span className="text-[10px] font-mono text-text-muted">
                            {formatRelativeTime(event.event_time)}
                          </span>
                          <span className="text-border text-[10px]">·</span>
                          <span className="text-[10px] text-text-muted">
                            {formatDepth(event.depth_km)}
                          </span>
                          <span
                            className={`text-[9px] font-bold px-1 rounded border tracking-widest ${srcCls}`}
                          >
                            {event.source_network.toUpperCase()}
                          </span>
                          {event.is_live && (
                            <span className="text-[9px] font-bold text-accent tracking-widest">
                              CANLI
                            </span>
                          )}
                        </div>
                      </div>

                      {/* Quality indicator chip */}
                      <div
                        className="shrink-0 w-5 h-5 rounded flex items-center justify-center bg-surface-elevated border border-border"
                        title={`Kalite: ${event.quality_indicator}`}
                      >
                        <span
                          className={`text-[9px] font-bold font-mono ${
                            isReviewed(event.quality_indicator)
                              ? 'text-mag-green'
                              : 'text-text-muted'
                          }`}
                        >
                          {event.quality_indicator}
                        </span>
                      </div>
                    </Link>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>

      {/* ── Right: quality histogram ── */}
      <div className="w-[300px] shrink-0 flex flex-col min-h-0 bg-surface">
        <div className="px-4 py-2.5 border-b border-border shrink-0">
          <h2 className="text-[9px] font-semibold uppercase tracking-widest text-text-muted">
            Kalite & Büyüklük
          </h2>
        </div>
        <div className="p-5 overflow-y-auto flex-1">
          <QualityHistogram events={displayEvents} />
        </div>
      </div>

    </div>
  );
}

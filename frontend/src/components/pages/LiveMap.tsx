'use client';

import dynamic from 'next/dynamic';
import { useMemo } from 'react';
import Link from 'next/link';
import type { DisplayEvent, AlertEvent, BandStats } from '@/types';
import { getMagnitudeInfo, formatMagnitude, formatRelativeTime } from '@/lib/magnitude';

const EarthquakeMap = dynamic(() => import('@/components/MapInner'), {
  ssr: false,
  loading: () => (
    <div className="w-full h-full bg-[#12151d] flex items-center justify-center text-[#575c6e] text-sm">
      Harita yükleniyor…
    </div>
  ),
});

const TURKEY_BBOX = { minLon: 25, maxLon: 45, minLat: 35, maxLat: 43 };

function inTurkey(e: DisplayEvent) {
  return (
    e.longitude >= TURKEY_BBOX.minLon && e.longitude <= TURKEY_BBOX.maxLon &&
    e.latitude  >= TURKEY_BBOX.minLat && e.latitude  <= TURKEY_BBOX.maxLat
  );
}

function riskFromBands(bands: BandStats[]): { color: string; label: string; textClass: string } {
  const m = Object.fromEntries(bands.map((b) => [b.band, b]));
  if ((m['major']?.count_24h ?? 0) > 0 || (m['strong']?.count_24h ?? 0) > 0)
    return { color: '#ef4444', label: 'TEHLİKE', textClass: 'text-red-400' };
  if ((m['moderate']?.count_24h ?? 0) > 0)
    return { color: '#f97316', label: 'UYARI', textClass: 'text-orange-400' };
  if ((m['light']?.count_24h ?? 0) > 0)
    return { color: '#eab308', label: 'AKTİF', textClass: 'text-yellow-400' };
  return { color: '#22c55e', label: 'NORMAL', textClass: 'text-emerald-400' };
}

interface Props {
  events: DisplayEvent[];
  bands: BandStats[];
  alerts: AlertEvent[];
  filterTurkey: boolean;
  minMagnitude: number;
  onFilterTurkey: () => void;
  onMinMagnitude: (v: number) => void;
  loading: boolean;
}

export function LiveMap({ events, bands, alerts, filterTurkey, minMagnitude, onFilterTurkey, onMinMagnitude, loading }: Props) {
  const geoFiltered = useMemo(
    () => (filterTurkey ? events.filter(inTurkey) : events),
    [events, filterTurkey],
  );
  const filtered = useMemo(
    () => geoFiltered.filter((e) => (e.ml_magnitude ?? e.magnitude) >= minMagnitude),
    [geoFiltered, minMagnitude],
  );

  const risk = riskFromBands(bands);

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      {/* Risk banner */}
      <div
        className="flex items-center gap-3 px-4 py-2 shrink-0 border-b border-[#232736]"
        style={{ backgroundColor: `${risk.color}18` }}
      >
        <span className="w-2 h-2 rounded-full shrink-0 animate-pulse" style={{ backgroundColor: risk.color }} />
        <span className={`text-[10px] font-bold tracking-[0.15em] ${risk.textClass}`}>{risk.label}</span>
        <span className="text-[10px] text-[#575c6e]">
          Son 24 saatte{' '}
          <span className="font-mono text-[#8b90a2]">
            {bands.reduce((s, b) => s + b.count_24h, 0)}
          </span>{' '}
          deprem
        </span>
        {alerts.length > 0 && (
          <span className="ml-auto text-[10px] font-bold text-red-400 tracking-widest">
            ⚠ {alerts.length} AKTİF UYARI
          </span>
        )}
      </div>

      <div className="flex flex-1 min-h-0 overflow-hidden">
        {/* Map */}
        <div className="flex-1 min-h-0 relative">
          {/* Controls overlay */}
          <div className="absolute top-3 left-3 z-10 flex flex-col gap-2">
            <button
              onClick={onFilterTurkey}
              className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded text-[10px] font-bold tracking-widest border backdrop-blur-sm transition-all ${
                filterTurkey
                  ? 'bg-[#4a90e2]/20 border-[#4a90e2]/60 text-[#4a90e2]'
                  : 'bg-[#0d0f14]/80 border-[#232736] text-[#575c6e] hover:text-[#8b90a2]'
              }`}
            >
              TÜRKİYE
              {filterTurkey && <span className="w-1.5 h-1.5 rounded-full bg-[#4a90e2]" />}
            </button>
            <div className="flex items-center gap-2 px-2.5 py-1.5 rounded border border-[#232736] bg-[#0d0f14]/80 backdrop-blur-sm">
              <span className="text-[10px] font-bold tracking-widest text-[#575c6e]">M ≥</span>
              <span className="text-[10px] font-bold font-mono text-[#e2e4ed] w-6 text-center tabular-nums">
                {minMagnitude.toFixed(1)}
              </span>
              <input
                type="range" min={0} max={7} step={0.5} value={minMagnitude}
                onChange={(e) => onMinMagnitude(parseFloat(e.target.value))}
                className="w-24 h-1 accent-[#4a90e2] cursor-pointer"
              />
            </div>
          </div>

          {loading ? (
            <div className="w-full h-full bg-[#12151d] flex items-center justify-center text-[#575c6e] text-sm">
              Yükleniyor…
            </div>
          ) : (
            <EarthquakeMap events={filtered} />
          )}
        </div>

        {/* Event list overlay */}
        <div className="w-[300px] shrink-0 border-l border-[#232736] bg-[#12151d] flex flex-col min-h-0">
          <div className="flex items-center justify-between px-4 py-2.5 border-b border-[#232736] shrink-0">
            <h2 className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">
              Son Depremler
            </h2>
            <span className="text-[9px] font-mono text-[#575c6e]">
              {filterTurkey ? `${filtered.length} / ${events.length}` : filtered.length}
            </span>
          </div>
          <div className="flex-1 overflow-y-auto overscroll-contain">
            {filtered.slice(0, 60).map((event) => {
              const mag = event.ml_magnitude ?? event.magnitude;
              const info = getMagnitudeInfo(mag);
              return (
                <Link
                  key={event.source_id}
                  href={`/deprem/${encodeURIComponent(event.source_id)}`}
                  prefetch={false}
                  className="flex items-center gap-3 px-4 py-2.5 border-b border-[#232736]/60 hover:bg-[#1a1e2b] transition-colors"
                >
                  <div className={`w-11 text-center rounded py-0.5 border ${info.bgClass} ${info.borderClass} shrink-0`}>
                    <span className={`text-sm font-bold font-mono ${info.textClass}`}>
                      {formatMagnitude(mag)}
                    </span>
                  </div>
                  <div className="flex-1 min-w-0">
                    <p className="text-xs text-[#e2e4ed] truncate">{event.region_name ?? event.source_id}</p>
                    <p className="text-[10px] text-[#575c6e] font-mono mt-0.5">
                      {formatRelativeTime(event.event_time)}
                      {event.is_live && (
                        <span className="ml-1.5 text-[#4a90e2] font-bold tracking-widest">CANLI</span>
                      )}
                    </p>
                  </div>
                </Link>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

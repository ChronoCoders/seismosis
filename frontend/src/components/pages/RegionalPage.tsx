'use client';

import { useMemo, useState } from 'react';
import type { DisplayEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

const TURKEY_BBOX = { minLon: 25, maxLon: 45, minLat: 35, maxLat: 43 };

function inTurkey(e: DisplayEvent) {
  return (
    e.longitude >= TURKEY_BBOX.minLon && e.longitude <= TURKEY_BBOX.maxLon &&
    e.latitude  >= TURKEY_BBOX.minLat && e.latitude  <= TURKEY_BBOX.maxLat
  );
}

type Period = '1h' | '24h' | '7d';
const PERIODS: { value: Period; label: string; ms: number }[] = [
  { value: '1h',  label: 'Son 1 Saat',  ms: 3_600_000       },
  { value: '24h', label: 'Son 24 Saat', ms: 86_400_000      },
  { value: '7d',  label: 'Son 7 Gün',   ms: 604_800_000     },
];

const SOURCE_COLORS: Record<string, string> = {
  USGS: 'bg-blue-500',
  EMSC: 'bg-purple-500',
  AFAD: 'bg-teal-500',
};

interface Props {
  events: DisplayEvent[];
}

export function RegionalPage({ events: allEvents }: Props) {
  const [period, setPeriod] = useState<Period>('24h');
  const [turkeyOnly, setTurkeyOnly] = useState(true);

  const events = useMemo(() => {
    const cutoffMs = PERIODS.find((p) => p.value === period)!.ms;
    const cutoff = Date.now() - cutoffMs;
    return allEvents.filter((e) => {
      const t = new Date(e.event_time).getTime();
      if (t < cutoff) return false;
      if (turkeyOnly && !inTurkey(e)) return false;
      return true;
    });
  }, [allEvents, period, turkeyOnly]);

  const regionData = useMemo(() => {
    const map = new Map<string, { count: number; maxMag: number; sources: Record<string, number> }>();
    for (const e of events) {
      const region = e.region_name ?? 'Bilinmeyen Bölge';
      const existing = map.get(region) ?? { count: 0, maxMag: 0, sources: {} };
      const mag = e.ml_magnitude ?? e.magnitude;
      existing.count++;
      if (mag > existing.maxMag) existing.maxMag = mag;
      existing.sources[e.source_network] = (existing.sources[e.source_network] ?? 0) + 1;
      map.set(region, existing);
    }
    return Array.from(map.entries())
      .map(([name, data]) => ({ name, ...data }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 15);
  }, [events]);

  const maxCount = Math.max(...regionData.map((r) => r.count), 1);

  const sourceSummary = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of events) {
      counts[e.source_network] = (counts[e.source_network] ?? 0) + 1;
    }
    return Object.entries(counts).sort((a, b) => b[1] - a[1]);
  }, [events]);

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-4 px-5 h-12 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div>
          <h1 className="text-sm font-semibold text-[#e2e4ed]">Bölgesel Analiz</h1>
          <p className="text-[10px] text-[#575c6e] mt-0.5">{events.length} olay</p>
        </div>
        <div className="flex items-center gap-1 rounded border border-[#232736] bg-[#0d0f14] overflow-hidden">
          {PERIODS.map((p) => (
            <button
              key={p.value}
              onClick={() => setPeriod(p.value)}
              className={`px-3 py-1.5 text-xs font-semibold transition-colors ${
                period === p.value ? 'bg-[#4a90e2]/20 text-[#4a90e2]' : 'text-[#575c6e] hover:text-[#8b90a2]'
              }`}
            >
              {p.label}
            </button>
          ))}
        </div>
        <button
          onClick={() => setTurkeyOnly((f) => !f)}
          className={`flex items-center gap-1.5 px-3 py-1.5 rounded border text-xs font-semibold transition-colors ${
            turkeyOnly
              ? 'bg-[#4a90e2]/20 border-[#4a90e2]/60 text-[#4a90e2]'
              : 'border-[#232736] text-[#575c6e] hover:text-[#8b90a2]'
          }`}
        >
          Türkiye Filtresi
          {turkeyOnly && <span className="w-1.5 h-1.5 rounded-full bg-[#4a90e2]" />}
        </button>
      </div>

      <div className="flex flex-1 min-h-0 overflow-hidden">
        {/* Main: active regions */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-4">
            En Aktif Bölgeler (Olay Sayısı)
          </p>

          {regionData.length === 0 ? (
            <div className="flex items-center justify-center h-40 text-[#575c6e] text-sm">
              Bu dönem için veri yok
            </div>
          ) : (
            <div className="space-y-3">
              {regionData.map((region, i) => {
                const info = getMagnitudeInfo(region.maxMag);
                const barPct = (region.count / maxCount) * 100;
                return (
                  <div key={region.name} className="rounded border border-[#232736] bg-[#12151d] px-4 py-3">
                    <div className="flex items-start gap-3">
                      <span className="text-[10px] font-bold font-mono text-[#575c6e] w-5 shrink-0 mt-0.5">
                        {i + 1}
                      </span>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center justify-between gap-2">
                          <p className="text-xs font-semibold text-[#e2e4ed] truncate" title={region.name}>
                            {region.name}
                          </p>
                          <div className="flex items-center gap-2 shrink-0">
                            <span className={`text-xs font-bold font-mono ${info.textClass}`}>
                              M {formatMagnitude(region.maxMag)}
                            </span>
                            <span className="text-xs font-bold font-mono text-[#e2e4ed]">
                              {region.count}
                            </span>
                          </div>
                        </div>
                        {/* Bar */}
                        <div className="mt-2 h-1.5 bg-[#1a1e2b] rounded overflow-hidden">
                          <div
                            className="h-full rounded bg-[#4a90e2]/60 transition-all duration-500"
                            style={{ width: `${barPct}%` }}
                          />
                        </div>
                        {/* Source chips */}
                        <div className="flex items-center gap-1.5 mt-2 flex-wrap">
                          {Object.entries(region.sources).sort((a, b) => b[1] - a[1]).map(([src, cnt]) => (
                            <span
                              key={src}
                              className="text-[8px] font-bold px-1.5 py-0.5 rounded font-mono bg-[#1a1e2b] text-[#575c6e]"
                            >
                              {src} {cnt}
                            </span>
                          ))}
                        </div>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Right sidebar: source breakdown */}
        <div className="w-[220px] shrink-0 border-l border-[#232736] bg-[#12151d] flex flex-col">
          <div className="px-4 py-2.5 border-b border-[#232736] shrink-0">
            <h2 className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">
              Kaynak Dağılımı
            </h2>
          </div>
          <div className="p-4 space-y-4">
            {sourceSummary.length === 0 ? (
              <p className="text-xs text-[#575c6e] text-center mt-4">Veri yok</p>
            ) : (
              sourceSummary.map(([src, cnt]) => {
                const pct = events.length > 0 ? (cnt / events.length) * 100 : 0;
                const dotCls = SOURCE_COLORS[src.toUpperCase()] ?? 'bg-zinc-500';
                return (
                  <div key={src}>
                    <div className="flex items-center justify-between mb-1.5">
                      <div className="flex items-center gap-2">
                        <div className={`w-2 h-2 rounded-sm ${dotCls}`} />
                        <span className="text-xs font-semibold text-[#8b90a2]">{src}</span>
                      </div>
                      <div className="text-right">
                        <span className="text-xs font-bold font-mono text-[#e2e4ed]">{cnt}</span>
                        <span className="text-[9px] text-[#575c6e] ml-1">({pct.toFixed(0)}%)</span>
                      </div>
                    </div>
                    <div className="h-1.5 bg-[#1a1e2b] rounded overflow-hidden">
                      <div
                        className={`h-full rounded ${dotCls} opacity-70 transition-all duration-500`}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  </div>
                );
              })
            )}
          </div>

          {/* Summary stats */}
          <div className="mt-auto border-t border-[#232736] p-4 space-y-3">
            <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e]">Özet</p>
            <div className="space-y-2">
              <div className="flex justify-between text-xs">
                <span className="text-[#575c6e]">Toplam</span>
                <span className="font-mono font-bold text-[#e2e4ed]">{events.length}</span>
              </div>
              <div className="flex justify-between text-xs">
                <span className="text-[#575c6e]">Bölge sayısı</span>
                <span className="font-mono font-bold text-[#e2e4ed]">{regionData.length}</span>
              </div>
              <div className="flex justify-between text-xs">
                <span className="text-[#575c6e]">En aktif</span>
                <span className="font-mono font-bold text-[#4a90e2] truncate max-w-[100px] text-right" title={regionData[0]?.name}>
                  {regionData[0]?.count ?? 0} olay
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

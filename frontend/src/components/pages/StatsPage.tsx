'use client';

import { useMemo, useState } from 'react';
import type { BandStats, DisplayEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

const BAND_ORDER = ['minor', 'light', 'moderate', 'strong', 'major'];
const BAND_LABELS: Record<string, string> = {
  minor:    'M < 2 (Mikro)',
  light:    'M 2–4 (Hafif)',
  moderate: 'M 4–6 (Orta)',
  strong:   'M 6–8 (Güçlü)',
  major:    'M 8+ (Büyük)',
};
const BAND_MID: Record<string, number> = {
  minor: 0.1, light: 2.1, moderate: 4.1, strong: 6.1, major: 8.1,
};

type Period = '24h' | '7d' | '30d';

const SOURCE_COLORS: Record<string, string> = {
  USGS: '#60a5fa',
  EMSC: '#c084fc',
  AFAD: '#2dd4bf',
};

function HourlyChart({ events }: { events: DisplayEvent[] }) {
  const buckets = useMemo(() => {
    const now = Date.now();
    return Array.from({ length: 24 }, (_, i) => {
      const start = now - (24 - i) * 3_600_000;
      const end   = start + 3_600_000;
      return events.filter((e) => {
        const t = new Date(e.event_time).getTime();
        return t >= start && t < end;
      }).length;
    });
  }, [events]);

  const max = Math.max(...buckets, 1);
  const now = new Date();

  return (
    <div>
      <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-4">
        Saatlik Aktivite (Son 24 Saat)
      </p>
      <div className="flex items-end gap-0.5 h-24">
        {buckets.map((cnt, i) => (
          <div
            key={i}
            className="flex-1 flex items-end h-full"
            title={`${new Date(now.getTime() - (23 - i) * 3_600_000).getHours()}:00 — ${cnt} deprem`}
          >
            <div
              className="w-full rounded-sm bg-[#4a90e2]/60 hover:bg-[#4a90e2] transition-colors"
              style={{
                height: cnt > 0 ? `${Math.max((cnt / max) * 100, 4)}%` : '2px',
                opacity: cnt > 0 ? 1 : 0.2,
              }}
            />
          </div>
        ))}
      </div>
      <div className="flex justify-between mt-1.5 text-[9px] text-[#575c6e] font-mono">
        <span>-24s</span>
        <span>-12s</span>
        <span>şimdi</span>
      </div>
    </div>
  );
}

interface Props {
  bands: BandStats[];
  events: DisplayEvent[];
}

export function StatsPage({ bands, events }: Props) {
  const [period, setPeriod] = useState<Period>('24h');

  const getPeriodCount  = (b: BandStats) => period === '24h' ? b.count_24h  : period === '7d' ? b.count_7d  : b.count_30d;
  const getPeriodMaxMag = (b: BandStats) => period === '24h' ? b.max_mag_24h : period === '7d' ? b.max_mag_7d : b.max_mag_30d;

  const totalEvents = useMemo(() => bands.reduce((s, b) => s + getPeriodCount(b), 0), [bands, period]);
  const overallMax = useMemo(() => bands.reduce((m, b) => Math.max(m, getPeriodMaxMag(b) ?? 0), 0), [bands, period]);
  const hours = period === '24h' ? 24 : period === '7d' ? 168 : 720;
  const rate = hours > 0 ? totalEvents / hours : 0;

  const bandRows = useMemo(() =>
    BAND_ORDER.map((key) => {
      const b = bands.find((x) => x.band === key);
      return { key, count: b ? getPeriodCount(b) : 0, maxMag: b ? (getPeriodMaxMag(b) ?? 0) : 0 };
    }),
  [bands, period]);

  const maxBandCount = Math.max(...bandRows.map((r) => r.count), 1);

  // Source breakdown from live events
  const sourceBreakdown = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of events) counts[e.source_network] = (counts[e.source_network] ?? 0) + 1;
    return Object.entries(counts).sort((a, b) => b[1] - a[1]);
  }, [events]);

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-4 px-5 py-3 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div>
          <h1 className="text-sm font-semibold text-[#e2e4ed]">İstatistikler</h1>
          <p className="text-[10px] text-[#575c6e] mt-0.5">Platform geneli aktivite özeti</p>
        </div>
        <div className="flex items-center gap-1 rounded border border-[#232736] bg-[#0d0f14] overflow-hidden">
          {(['24h', '7d', '30d'] as Period[]).map((p) => (
            <button
              key={p}
              onClick={() => setPeriod(p)}
              className={`px-3 py-1.5 text-xs font-semibold transition-colors ${
                period === p ? 'bg-[#4a90e2]/20 text-[#4a90e2]' : 'text-[#575c6e] hover:text-[#8b90a2]'
              }`}
            >
              {p === '24h' ? '24 Saat' : p === '7d' ? '7 Gün' : '30 Gün'}
            </button>
          ))}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-5 space-y-5">
        {/* KPI row */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          {[
            { label: 'Toplam Deprem',       value: totalEvents.toLocaleString('tr-TR'),                                                           sub: `Son ${period}`,       cls: 'text-[#e2e4ed]' },
            { label: 'En Yüksek Büyüklük',  value: overallMax > 0 ? `M ${formatMagnitude(overallMax)}` : '—',                                     sub: `Son ${period}`,       cls: overallMax > 0 ? getMagnitudeInfo(overallMax).textClass : 'text-[#575c6e]' },
            { label: 'Saatlik Oran',         value: `${rate.toFixed(2)}/sa`,                                                                        sub: `${hours}sa ortalaması`, cls: 'text-[#e2e4ed]' },
            { label: 'Veri Kaynağı',         value: sourceBreakdown.length.toString(),                                                              sub: 'Aktif kaynak',        cls: 'text-[#e2e4ed]' },
          ].map((kpi) => (
            <div key={kpi.label} className="rounded border border-[#232736] bg-[#12151d] px-4 py-3">
              <p className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">{kpi.label}</p>
              <p className={`text-2xl font-bold font-mono mt-2 ${kpi.cls}`}>{kpi.value}</p>
              <p className="text-[9px] text-[#575c6e] mt-1">{kpi.sub}</p>
            </div>
          ))}
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
          {/* Magnitude distribution */}
          <div className="rounded border border-[#232736] bg-[#12151d] p-5">
            <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-5">
              Büyüklük Dağılımı
            </p>
            <div className="space-y-3">
              {bandRows.map((row) => {
                const info = getMagnitudeInfo(BAND_MID[row.key]);
                const pct = (row.count / maxBandCount) * 100;
                return (
                  <div key={row.key}>
                    <div className="flex items-center justify-between mb-1.5">
                      <span className={`text-xs font-mono font-semibold ${info.textClass}`}>
                        {BAND_LABELS[row.key]}
                      </span>
                      <div className="flex items-center gap-3 text-[10px] font-mono">
                        <span className="text-[#8b90a2]">{row.count.toLocaleString('tr-TR')}</span>
                        {row.maxMag > 0 && (
                          <span className={`${info.textClass}`}>maks M {formatMagnitude(row.maxMag)}</span>
                        )}
                      </div>
                    </div>
                    <div className="h-2 bg-[#1a1e2b] rounded overflow-hidden">
                      <div
                        className="h-full rounded transition-all duration-500"
                        style={{ width: `${pct}%`, backgroundColor: info.dotColor, opacity: 0.75 }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Source breakdown */}
          <div className="rounded border border-[#232736] bg-[#12151d] p-5">
            <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-5">
              Kaynak Dağılımı (Bellek içi)
            </p>
            {sourceBreakdown.length === 0 ? (
              <div className="flex items-center justify-center h-24 text-[#575c6e] text-sm">Veri bekleniyor…</div>
            ) : (
              <div className="space-y-4">
                {sourceBreakdown.map(([src, cnt]) => {
                  const total = sourceBreakdown.reduce((s, [, c]) => s + c, 0);
                  const pct = total > 0 ? (cnt / total) * 100 : 0;
                  const color = SOURCE_COLORS[src.toUpperCase()] ?? '#8b90a2';
                  return (
                    <div key={src}>
                      <div className="flex items-center justify-between mb-1.5">
                        <div className="flex items-center gap-2">
                          <div className="w-2.5 h-2.5 rounded-sm" style={{ backgroundColor: color }} />
                          <span className="text-xs font-semibold text-[#8b90a2]">{src}</span>
                        </div>
                        <div className="text-right text-[10px] font-mono">
                          <span className="text-[#e2e4ed] font-bold">{cnt}</span>
                          <span className="text-[#575c6e] ml-1">({pct.toFixed(0)}%)</span>
                        </div>
                      </div>
                      <div className="h-2 bg-[#1a1e2b] rounded overflow-hidden">
                        <div
                          className="h-full rounded transition-all duration-500 opacity-75"
                          style={{ width: `${pct}%`, backgroundColor: color }}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        {/* Hourly chart */}
        <div className="rounded border border-[#232736] bg-[#12151d] p-5">
          <HourlyChart events={events} />
        </div>
      </div>
    </div>
  );
}

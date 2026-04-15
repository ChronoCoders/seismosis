'use client';

import { useState, useEffect, useCallback, useMemo } from 'react';
import { fetchEvents } from '@/lib/api';
import type { EarthquakeEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

// ── Config ────────────────────────────────────────────────────────────────────

type PeriodKey = '24h' | '7d' | '30d';

const PERIODS: { value: PeriodKey; label: string; hours: number }[] = [
  { value: '24h', label: '24 Saat', hours: 24  },
  { value: '7d',  label: '7 Gün',  hours: 168 },
  { value: '30d', label: '30 Gün', hours: 720 },
];

const HIST_BANDS = [
  { key: 'minor',    label: 'M < 2',  midMag: 0.1, maxMag: 2   },
  { key: 'light',    label: 'M 2–4',  midMag: 2.1, maxMag: 4   },
  { key: 'moderate', label: 'M 4–6',  midMag: 4.1, maxMag: 6   },
  { key: 'strong',   label: 'M 6–8',  midMag: 6.1, maxMag: 8   },
  { key: 'major',    label: 'M 8+',   midMag: 8.1, maxMag: 999 },
];

function bandIdx(mag: number): number {
  const i = HIST_BANDS.findIndex((b) => mag < b.maxMag);
  return i === -1 ? HIST_BANDS.length - 1 : i;
}

function periodRange(hours: number, offset: number) {
  // offset=0 → current period, offset=1 → immediately preceding period
  const end   = new Date(Date.now() - offset * hours * 3_600_000);
  const start = new Date(end.getTime() - hours * 3_600_000);
  return { start, end };
}

// ── Stats computation ─────────────────────────────────────────────────────────

interface PeriodStats {
  count: number;
  maxMag: number;
  avgMag: number;
  reviewed: number;
  automatic: number;
  bandCounts: number[];
  rate: number; // events per hour
}

function computeStats(events: EarthquakeEvent[], hours: number): PeriodStats {
  const count    = events.length;
  const maxMag   = events.reduce((m, e) => Math.max(m, e.magnitude), 0);
  const avgMag   = count > 0 ? events.reduce((s, e) => s + e.magnitude, 0) / count : 0;
  const reviewed = events.filter((e) => e.quality_indicator === 'A' || e.quality_indicator === 'B').length;
  const bandCounts = HIST_BANDS.map((_, i) =>
    events.filter((e) => bandIdx(e.magnitude) === i).length,
  );
  return {
    count, maxMag, avgMag, reviewed,
    automatic: count - reviewed,
    bandCounts,
    rate: hours > 0 ? count / hours : 0,
  };
}

// ── Stat comparison card ──────────────────────────────────────────────────────

function StatCard({
  label,
  valA,
  valB,
  format,
  colorA = 'text-accent',
  colorB = 'text-mag-orange',
  higherIsWorse = true,
}: {
  label: string;
  valA: number;
  valB: number;
  format?: (v: number) => string;
  colorA?: string;
  colorB?: string;
  higherIsWorse?: boolean;
}) {
  const fmt = format ?? ((v: number) => v.toLocaleString('tr-TR'));
  const delta = valA - valB;
  const pct   = valB > 0 ? (delta / valB) * 100 : null;

  let deltaColor = 'text-text-muted';
  if (delta !== 0) {
    deltaColor =
      (delta > 0) === higherIsWorse ? 'text-mag-red' : 'text-mag-green';
  }

  return (
    <div className="rounded border border-border bg-surface p-4">
      <p className="text-[9px] font-semibold uppercase tracking-widest text-text-muted mb-3">
        {label}
      </p>
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <p className="text-[9px] text-text-muted mb-1">Dönem A</p>
          <p className={`text-2xl font-bold font-mono leading-none ${colorA}`}>
            {fmt(valA)}
          </p>
        </div>
        <div className="flex-1">
          <p className="text-[9px] text-text-muted mb-1">Dönem B</p>
          <p className={`text-2xl font-bold font-mono leading-none ${colorB}`}>
            {fmt(valB)}
          </p>
        </div>
        <div className="text-right shrink-0">
          <p className="text-[9px] text-text-muted mb-1">Fark</p>
          <p className={`text-sm font-bold font-mono leading-none ${deltaColor}`}>
            {delta >= 0 ? '+' : ''}{fmt(delta)}
          </p>
          {pct !== null && (
            <p className={`text-[9px] font-mono mt-0.5 ${deltaColor}`}>
              {pct >= 0 ? '+' : ''}{pct.toFixed(1)}%
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Magnitude distribution comparison chart ───────────────────────────────────

function ComparisonChart({
  statsA,
  statsB,
  labelA,
  labelB,
}: {
  statsA: PeriodStats;
  statsB: PeriodStats;
  labelA: string;
  labelB: string;
}) {
  const maxVal = Math.max(...statsA.bandCounts, ...statsB.bandCounts, 1);

  return (
    <div>
      <p className="text-[9px] font-semibold uppercase tracking-widest text-text-muted mb-5">
        Büyüklük Dağılımı Karşılaştırması
      </p>

      <div className="space-y-5">
        {HIST_BANDS.map((band, i) => {
          const info = getMagnitudeInfo(band.midMag);
          const pctA = (statsA.bandCounts[i] / maxVal) * 100;
          const pctB = (statsB.bandCounts[i] / maxVal) * 100;
          return (
            <div key={band.key}>
              <div className="flex items-center justify-between mb-1.5">
                <span className={`text-xs font-mono font-semibold ${info.textClass}`}>
                  {band.label}
                </span>
                <div className="flex items-center gap-2 text-[10px] font-mono">
                  <span className="text-accent">{statsA.bandCounts[i]}</span>
                  <span className="text-border">/</span>
                  <span className="text-mag-orange">{statsB.bandCounts[i]}</span>
                </div>
              </div>
              {/* A bar */}
              <div className="h-3.5 bg-surface-elevated rounded overflow-hidden mb-1">
                <div
                  className="h-full rounded bg-accent/70 transition-all duration-500"
                  style={{ width: `${pctA}%` }}
                  title={`${labelA}: ${statsA.bandCounts[i]}`}
                />
              </div>
              {/* B bar */}
              <div className="h-3.5 bg-surface-elevated rounded overflow-hidden">
                <div
                  className="h-full rounded bg-mag-orange/70 transition-all duration-500"
                  style={{ width: `${pctB}%` }}
                  title={`${labelB}: ${statsB.bandCounts[i]}`}
                />
              </div>
            </div>
          );
        })}
      </div>

      {/* Legend */}
      <div className="flex items-center gap-5 mt-5 pt-4 border-t border-border/40">
        <div className="flex items-center gap-1.5">
          <div className="w-3 h-3 rounded-sm bg-accent/70" />
          <span className="text-[10px] text-text-muted">{labelA}</span>
        </div>
        <div className="flex items-center gap-1.5">
          <div className="w-3 h-3 rounded-sm bg-mag-orange/70" />
          <span className="text-[10px] text-text-muted">{labelB}</span>
        </div>
      </div>
    </div>
  );
}

// ── Period selector widget ────────────────────────────────────────────────────

function PeriodSelector({
  label,
  value,
  onChange,
  accentClass,
}: {
  label: string;
  value: PeriodKey;
  onChange: (v: PeriodKey) => void;
  accentClass: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-xs text-text-muted shrink-0">{label}:</span>
      <div className="flex items-center gap-1 rounded border border-border bg-bg overflow-hidden">
        {PERIODS.map((p) => (
          <button
            key={p.value}
            onClick={() => onChange(p.value)}
            className={`px-3 py-1.5 text-xs font-semibold transition-colors ${
              value === p.value
                ? accentClass
                : 'text-text-muted hover:text-text-secondary'
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  );
}

// ── Compare tab ───────────────────────────────────────────────────────────────

export default function CompareTab() {
  const [periodA, setPeriodA] = useState<PeriodKey>('7d');
  const [periodB, setPeriodB] = useState<PeriodKey>('7d');
  const [eventsA, setEventsA] = useState<EarthquakeEvent[]>([]);
  const [eventsB, setEventsB] = useState<EarthquakeEvent[]>([]);
  const [loadingA, setLoadingA] = useState(true);
  const [loadingB, setLoadingB] = useState(true);

  const loadPeriod = useCallback(
    async (key: PeriodKey, offset: number): Promise<EarthquakeEvent[]> => {
      const hours = PERIODS.find((p) => p.value === key)!.hours;
      const { start, end } = periodRange(hours, offset);
      const resp = await fetchEvents({
        page_size: 1000,
        page: 1,
        start_time: start.toISOString(),
        end_time: end.toISOString(),
      });
      return resp.events;
    },
    [],
  );

  useEffect(() => {
    setLoadingA(true);
    loadPeriod(periodA, 0)
      .then(setEventsA)
      .catch(console.error)
      .finally(() => setLoadingA(false));
  }, [periodA, loadPeriod]);

  useEffect(() => {
    setLoadingB(true);
    loadPeriod(periodB, 1)
      .then(setEventsB)
      .catch(console.error)
      .finally(() => setLoadingB(false));
  }, [periodB, loadPeriod]);

  const hoursA = PERIODS.find((p) => p.value === periodA)!.hours;
  const hoursB = PERIODS.find((p) => p.value === periodB)!.hours;

  const statsA = useMemo(() => computeStats(eventsA, hoursA), [eventsA, hoursA]);
  const statsB = useMemo(() => computeStats(eventsB, hoursB), [eventsB, hoursB]);

  const labelA = `${PERIODS.find((p) => p.value === periodA)!.label} (güncel)`;
  const labelB = `${PERIODS.find((p) => p.value === periodB)!.label} (önceki)`;

  const loading = loadingA || loadingB;

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">

      {/* Controls */}
      <div className="flex items-center gap-6 px-5 py-3 border-b border-border bg-surface shrink-0 flex-wrap">
        <div className="flex items-center gap-1.5">
          <div className="w-2.5 h-2.5 rounded-sm bg-accent/70 shrink-0" />
          <PeriodSelector
            label="Dönem A (güncel)"
            value={periodA}
            onChange={setPeriodA}
            accentClass="bg-accent/20 text-accent"
          />
        </div>
        <div className="flex items-center gap-1.5">
          <div className="w-2.5 h-2.5 rounded-sm bg-mag-orange/70 shrink-0" />
          <PeriodSelector
            label="Dönem B (önceki)"
            value={periodB}
            onChange={setPeriodB}
            accentClass="bg-orange-950/60 text-mag-orange"
          />
        </div>
        {!loading && (
          <div className="ml-auto text-[10px] font-mono text-text-muted flex items-center gap-3">
            <span>
              <span className="text-accent">{statsA.count}</span> vs{' '}
              <span className="text-mag-orange">{statsB.count}</span> deprem
            </span>
          </div>
        )}
      </div>

      {/* Content */}
      {loading ? (
        <div className="flex items-center justify-center flex-1 text-text-muted text-sm">
          Yükleniyor…
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto p-5 space-y-5">

          {/* Stat cards row */}
          <div className="grid grid-cols-2 xl:grid-cols-4 gap-4">
            <StatCard
              label="Toplam Deprem"
              valA={statsA.count}
              valB={statsB.count}
              higherIsWorse={true}
            />
            <StatCard
              label="En Yüksek Büyüklük"
              valA={statsA.maxMag}
              valB={statsB.maxMag}
              format={(v) => (v > 0 ? `M ${formatMagnitude(v)}` : '—')}
              colorA={statsA.maxMag > 0 ? getMagnitudeInfo(statsA.maxMag).textClass : 'text-text-muted'}
              colorB={statsB.maxMag > 0 ? getMagnitudeInfo(statsB.maxMag).textClass : 'text-text-muted'}
              higherIsWorse={true}
            />
            <StatCard
              label="Ort. Büyüklük"
              valA={statsA.avgMag}
              valB={statsB.avgMag}
              format={(v) => (v > 0 ? `M ${formatMagnitude(v)}` : '—')}
              higherIsWorse={true}
            />
            <StatCard
              label="Saatlik Oran"
              valA={statsA.rate}
              valB={statsB.rate}
              format={(v) => `${v.toFixed(2)}/sa`}
              higherIsWorse={true}
            />
          </div>

          {/* Magnitude comparison chart */}
          <div className="rounded border border-border bg-surface p-5">
            <ComparisonChart
              statsA={statsA}
              statsB={statsB}
              labelA={labelA}
              labelB={labelB}
            />
          </div>

          {/* Quality breakdown row */}
          <div className="grid grid-cols-2 gap-4">
            <StatCard
              label="Gözden Geçirilmiş"
              valA={statsA.reviewed}
              valB={statsB.reviewed}
              colorA="text-mag-green"
              colorB="text-mag-green"
              higherIsWorse={false}
            />
            <StatCard
              label="Otomatik Çözüm"
              valA={statsA.automatic}
              valB={statsB.automatic}
              higherIsWorse={false}
            />
          </div>

        </div>
      )}
    </div>
  );
}

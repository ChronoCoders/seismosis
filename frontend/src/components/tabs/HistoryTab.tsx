'use client';

import { useState, useEffect, useCallback, useMemo } from 'react';
import Link from 'next/link';
import { fetchEvents } from '@/lib/api';
import type { EarthquakeEvent } from '@/types';
import { getMagnitudeInfo, formatMagnitude, formatDepth, formatDateTime } from '@/lib/magnitude';

// ── Period config ─────────────────────────────────────────────────────────────

type Period = '24h' | '7d' | '30d';

const PERIODS: { value: Period; label: string; hours: number }[] = [
  { value: '24h', label: 'Son 24 Saat', hours: 24  },
  { value: '7d',  label: 'Son 7 Gün',  hours: 168 },
  { value: '30d', label: 'Son 30 Gün', hours: 720 },
];

const TURKEY_BBOX = { minLon: 25, maxLon: 45, minLat: 35, maxLat: 43 };
const TABLE_PAGE_SIZE = 25;

// ── Trend chart ───────────────────────────────────────────────────────────────

function TrendChart({
  events,
  period,
}: {
  events: EarthquakeEvent[];
  period: Period;
}) {
  const { buckets, labels } = useMemo(() => {
    const cfg = PERIODS.find((p) => p.value === period)!;
    const bucketCount = period === '24h' ? 24 : period === '7d' ? 7 : 30;
    const bucketHours = cfg.hours / bucketCount;
    const now = Date.now();

    const buckets = Array.from({ length: bucketCount }, (_, i) => {
      const start = now - (bucketCount - i) * bucketHours * 3_600_000;
      const end   = start + bucketHours * 3_600_000;
      return events.filter((e) => {
        const t = new Date(e.event_time).getTime();
        return t >= start && t < end;
      }).length;
    });

    const labels = Array.from({ length: bucketCount }, (_, i) => {
      const t = new Date(now - (bucketCount - i) * bucketHours * 3_600_000);
      return period === '24h'
        ? `${t.getHours().toString().padStart(2, '0')}:00`
        : `${t.getDate()}/${t.getMonth() + 1}`;
    });

    return { buckets, labels };
  }, [events, period]);

  const maxCount = Math.max(...buckets, 1);

  return (
    <div>
      <p className="text-[9px] font-semibold uppercase tracking-widest text-text-muted mb-4">
        Olay Trendi
      </p>

      {/* Bars */}
      <div className="flex items-end gap-px h-28">
        {buckets.map((count, i) => (
          <div
            key={i}
            className="flex-1 flex items-end"
            title={`${labels[i]}: ${count}`}
          >
            <div
              className="w-full bg-accent/60 hover:bg-accent rounded-sm transition-colors"
              style={{
                height: count > 0 ? `${Math.max((count / maxCount) * 100, 4)}%` : '1px',
                opacity: count > 0 ? 1 : 0.2,
              }}
            />
          </div>
        ))}
      </div>

      {/* X-axis labels */}
      <div className="flex justify-between mt-1.5">
        <span className="text-[9px] text-text-muted font-mono">{labels[0]}</span>
        <span className="text-[9px] text-text-muted font-mono">
          {labels[Math.floor(labels.length / 2)]}
        </span>
        <span className="text-[9px] text-text-muted font-mono">
          {labels[labels.length - 1]}
        </span>
      </div>

      {/* Y-axis hint */}
      <div className="mt-3 text-[9px] text-text-muted font-mono">
        Maks: {maxCount} deprem / dilim
      </div>
    </div>
  );
}

// ── Event table ───────────────────────────────────────────────────────────────

function EventTable({
  events,
  page,
  onPage,
  total,
}: {
  events: EarthquakeEvent[];
  page: number;
  onPage: (p: number) => void;
  total: number;
}) {
  const totalPages = Math.ceil(events.length / TABLE_PAGE_SIZE);
  const slice = events.slice((page - 1) * TABLE_PAGE_SIZE, page * TABLE_PAGE_SIZE);

  return (
    <div>
      <table className="w-full text-xs border-collapse">
        <thead className="sticky top-0 bg-surface z-10">
          <tr className="border-b border-border text-left">
            {['Büyüklük', 'Konum', 'Zaman', 'Derinlik', 'Ağ', 'Kalite'].map((h) => (
              <th
                key={h}
                className="px-4 py-2.5 text-text-muted font-semibold text-[10px] uppercase tracking-wider"
              >
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {slice.map((event) => {
            const info = getMagnitudeInfo(event.magnitude);
            const reviewed =
              event.quality_indicator === 'A' || event.quality_indicator === 'B';
            return (
              <tr
                key={event.source_id}
                className="border-b border-border/40 hover:bg-surface-elevated transition-colors"
              >
                <td className="px-4 py-2 whitespace-nowrap">
                  <span className={`font-mono font-bold ${info.textClass}`}>
                    M {formatMagnitude(event.magnitude)}
                  </span>
                  <span className="ml-1.5 text-text-muted text-[10px]">
                    {event.magnitude_type}
                  </span>
                </td>
                <td className="px-4 py-2 max-w-[220px]">
                  <Link
                    href={`/deprem/${encodeURIComponent(event.source_id)}`}
                    className="text-text-primary hover:text-accent truncate block transition-colors"
                    title={event.region_name ?? event.source_id}
                  >
                    {event.region_name ?? event.source_id}
                  </Link>
                </td>
                <td className="px-4 py-2 font-mono text-text-secondary whitespace-nowrap text-[10px]">
                  {formatDateTime(event.event_time)}
                </td>
                <td className="px-4 py-2 font-mono text-text-muted">
                  {formatDepth(event.depth_km)}
                </td>
                <td className="px-4 py-2 font-mono text-text-muted uppercase text-[10px]">
                  {event.source_network}
                </td>
                <td className="px-4 py-2">
                  <span
                    className={`font-mono font-bold text-[10px] ${
                      reviewed ? 'text-mag-green' : 'text-text-muted'
                    }`}
                    title={
                      reviewed ? 'Gözden geçirilmiş' : 'Otomatik çözüm'
                    }
                  >
                    {event.quality_indicator}
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {/* Pagination footer */}
      <div className="flex items-center justify-between px-4 py-3 border-t border-border bg-surface sticky bottom-0">
        <span className="text-xs text-text-muted">
          {events.length.toLocaleString('tr-TR')} sonuç gösteriliyor
          {total > events.length && (
            <span className="ml-1 text-mag-yellow">
              (toplam {total.toLocaleString('tr-TR')})
            </span>
          )}
        </span>
        {totalPages > 1 && (
          <div className="flex items-center gap-1">
            <button
              onClick={() => onPage(Math.max(1, page - 1))}
              disabled={page === 1}
              className="px-2 py-1 text-xs rounded border border-border text-text-secondary hover:bg-surface-elevated disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            >
              ←
            </button>
            <span className="px-3 text-xs text-text-muted font-mono">
              {page} / {totalPages}
            </span>
            <button
              onClick={() => onPage(Math.min(totalPages, page + 1))}
              disabled={page === totalPages}
              className="px-2 py-1 text-xs rounded border border-border text-text-secondary hover:bg-surface-elevated disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            >
              →
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ── History tab ───────────────────────────────────────────────────────────────

export default function HistoryTab() {
  const [period, setPeriod]           = useState<Period>('7d');
  const [filterTurkey, setFilterTurkey] = useState(false);
  const [events, setEvents]           = useState<EarthquakeEvent[]>([]);
  const [total, setTotal]             = useState(0);
  const [loading, setLoading]         = useState(true);
  const [page, setPage]               = useState(1);

  const load = useCallback(async () => {
    setLoading(true);
    setPage(1);
    try {
      const hours = PERIODS.find((p) => p.value === period)!.hours;
      const endTime = new Date();
      const startTime = new Date(endTime.getTime() - hours * 3_600_000);

      const params: Record<string, string | number> = {
        page_size: 1000,
        page: 1,
        start_time: startTime.toISOString(),
        end_time:   endTime.toISOString(),
      };
      if (filterTurkey) {
        params.min_lat = TURKEY_BBOX.minLat;
        params.max_lat = TURKEY_BBOX.maxLat;
        params.min_lon = TURKEY_BBOX.minLon;
        params.max_lon = TURKEY_BBOX.maxLon;
      }

      const resp = await fetchEvents(params);
      // API returns newest-first; reverse for trend chart (oldest → newest)
      setEvents(resp.events);
      setTotal(resp.total);
    } catch (err) {
      console.error('HistoryTab load failed:', err);
    } finally {
      setLoading(false);
    }
  }, [period, filterTurkey]);

  useEffect(() => { void load(); }, [load]);

  const stats = useMemo(() => {
    const maxMag = events.reduce((m, e) => Math.max(m, e.magnitude), 0);
    const reviewed = events.filter(
      (e) => e.quality_indicator === 'A' || e.quality_indicator === 'B',
    ).length;
    return { maxMag, reviewed, automatic: events.length - reviewed };
  }, [events]);

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">

      {/* Controls bar */}
      <div className="flex items-center gap-4 px-5 py-3 border-b border-border bg-surface shrink-0 flex-wrap">
        {/* Period selector */}
        <div className="flex items-center gap-1 rounded border border-border bg-bg overflow-hidden">
          {PERIODS.map((p) => (
            <button
              key={p.value}
              onClick={() => setPeriod(p.value)}
              className={`px-3 py-1.5 text-xs font-semibold transition-colors ${
                period === p.value
                  ? 'bg-accent/20 text-accent'
                  : 'text-text-muted hover:text-text-secondary'
              }`}
            >
              {p.label}
            </button>
          ))}
        </div>

        {/* Turkey filter */}
        <button
          onClick={() => setFilterTurkey((f) => !f)}
          className={`flex items-center gap-1.5 px-3 py-1.5 rounded border text-xs font-semibold transition-colors ${
            filterTurkey
              ? 'bg-accent/20 border-accent/60 text-accent'
              : 'border-border text-text-muted hover:text-text-secondary'
          }`}
        >
          Türkiye Filtresi
          {filterTurkey && <span className="w-1.5 h-1.5 rounded-full bg-accent" />}
        </button>

        {/* Summary stats */}
        {!loading && (
          <div className="ml-auto flex items-center gap-4 text-xs font-mono text-text-muted">
            <span>{events.length.toLocaleString('tr-TR')} kayıt</span>
            {stats.maxMag > 0 && (
              <span className={getMagnitudeInfo(stats.maxMag).textClass}>
                maks M {formatMagnitude(stats.maxMag)}
              </span>
            )}
            <span className="text-mag-green">{stats.reviewed} gözd.</span>
            <span>{stats.automatic} otom.</span>
          </div>
        )}
      </div>

      {/* Content */}
      <div className="flex flex-1 min-h-0 overflow-hidden">

        {/* Trend chart sidebar */}
        <div className="w-[240px] shrink-0 border-r border-border flex flex-col p-5 bg-surface">
          {loading ? (
            <div className="flex items-center justify-center flex-1 text-text-muted text-sm">
              Yükleniyor…
            </div>
          ) : events.length === 0 ? (
            <div className="flex items-center justify-center flex-1 text-text-muted text-xs text-center">
              Bu dönem için veri yok
            </div>
          ) : (
            <TrendChart events={events} period={period} />
          )}
        </div>

        {/* Event table */}
        <div className="flex-1 min-h-0 overflow-y-auto">
          {loading ? (
            <div className="flex items-center justify-center h-32 text-text-muted text-sm">
              Yükleniyor…
            </div>
          ) : events.length === 0 ? (
            <div className="flex items-center justify-center h-32 text-text-muted text-sm">
              Bu dönem için veri bulunamadı
            </div>
          ) : (
            <EventTable
              events={events}
              page={page}
              onPage={setPage}
              total={total}
            />
          )}
        </div>

      </div>
    </div>
  );
}

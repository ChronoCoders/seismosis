'use client';

import { useState, useEffect, useCallback, useMemo } from 'react';
import dynamic from 'next/dynamic';
import Link from 'next/link';
import type {
  DisplayEvent,
  AlertEvent,
  EnrichedEvent,
  BandStats,
} from '@/types';
import { apiEventToDisplay, enrichedToDisplay } from '@/types';
import { fetchEvents, fetchStats } from '@/lib/api';
import { useWebSocket } from '@/hooks/useWebSocket';
import { Header } from '@/components/Header';
import { EarthquakeList } from '@/components/EarthquakeList';
import { MagnitudeChart } from '@/components/RegionalStats';
import {
  getMagnitudeInfo,
  formatMagnitude,
  formatRelativeTime,
} from '@/lib/magnitude';

const EarthquakeMap = dynamic(() => import('@/components/MapInner'), {
  ssr: false,
  loading: () => (
    <div className="w-full h-full bg-surface flex items-center justify-center text-text-muted text-sm">
      Harita yükleniyor…
    </div>
  ),
});

const WS_URL =
  typeof window !== 'undefined'
    ? (process.env.NEXT_PUBLIC_WS_URL ?? 'ws://localhost:9093')
    : '';

const MAX_EVENTS = 100;
const MAX_ALERTS = 10;

// Turkey bounding box for the filter toggle
const TURKEY_BBOX = { minLon: 25, maxLon: 45, minLat: 35, maxLat: 43 };

function inTurkey(e: DisplayEvent) {
  return (
    e.longitude >= TURKEY_BBOX.minLon &&
    e.longitude <= TURKEY_BBOX.maxLon &&
    e.latitude  >= TURKEY_BBOX.minLat &&
    e.latitude  <= TURKEY_BBOX.maxLat
  );
}

// ── Derived stats from BandStats ──────────────────────────────────────────────

function derivedStats(bands: BandStats[]) {
  const total24h  = bands.reduce((s, b) => s + b.count_24h, 0);
  const total1h   = bands.reduce((s, b) => s + b.count_1h,  0);
  const maxMag24h = bands.reduce((m, b) => Math.max(m, b.max_mag_24h ?? 0), 0);
  const rate24h   = total24h > 0 ? total24h / 24 : 0;
  return { total24h, total1h, maxMag24h, rate24h };
}

// ── Risk indicator from bands ─────────────────────────────────────────────────

function riskFromBands(bands: BandStats[]): { color: string; label: string } {
  const m = Object.fromEntries(bands.map((b) => [b.band, b]));
  if ((m['strong']?.count_24h ?? 0) > 0 || (m['major']?.count_24h ?? 0) > 0)
    return { color: '#ef4444', label: 'TEHLİKE' };
  if ((m['moderate']?.count_24h ?? 0) > 0)
    return { color: '#f97316', label: 'UYARI' };
  if ((m['light']?.count_24h ?? 0) > 0)
    return { color: '#eab308', label: 'AKTİF' };
  return { color: '#22c55e', label: 'NORMAL' };
}

// ── Top statistics bar ────────────────────────────────────────────────────────

interface StatTileProps {
  label: string;
  value: React.ReactNode;
  valueClass?: string;
  sub?: string;
}

function StatTile({ label, value, valueClass = 'text-text-primary', sub }: StatTileProps) {
  return (
    <div className="flex flex-col justify-center px-5 py-2 border-r border-border last:border-0 min-w-[120px]">
      <span className="text-[9px] font-semibold uppercase tracking-widest text-text-muted leading-none">
        {label}
      </span>
      <span className={`text-lg font-bold font-mono leading-tight mt-1 ${valueClass}`}>
        {value}
      </span>
      {sub && (
        <span className="text-[9px] text-text-muted mt-0.5 leading-none">{sub}</span>
      )}
    </div>
  );
}

function TopStatsBar({
  bands,
  events,
}: {
  bands: BandStats[];
  events: DisplayEvent[];
}) {
  if (bands.length === 0) return null;

  const { total24h, total1h, maxMag24h, rate24h } = derivedStats(bands);
  const reviewed = events.filter(
    (e) => e.quality_indicator === 'A' || e.quality_indicator === 'B',
  ).length;
  const automatic = events.length - reviewed;
  const maxInfo = getMagnitudeInfo(maxMag24h);

  return (
    <div className="flex items-stretch shrink-0 border-b border-border bg-surface overflow-x-auto">
      <StatTile
        label="Son 24 Saat"
        value={total24h.toLocaleString('tr-TR')}
        sub={`Son 1 saatte: ${total1h}`}
      />
      <StatTile
        label="En Yüksek Büyüklük"
        value={maxMag24h > 0 ? `M ${formatMagnitude(maxMag24h)}` : '—'}
        valueClass={maxMag24h > 0 ? maxInfo.textClass : 'text-text-muted'}
        sub="Son 24 saat"
      />
      <StatTile
        label="Ort. Sıklık"
        value={`${rate24h.toFixed(1)}/sa`}
        sub="24 saatlik oran"
      />
      <div className="flex flex-col justify-center px-5 py-2 min-w-[160px]">
        <span className="text-[9px] font-semibold uppercase tracking-widest text-text-muted leading-none">
          Kalite Dağılımı
        </span>
        <div className="flex items-center gap-3 mt-1">
          <div className="flex items-baseline gap-1">
            <span className="text-base font-bold font-mono text-mag-green">{reviewed}</span>
            <span className="text-[9px] text-text-muted">gözden geçirilmiş</span>
          </div>
          <span className="text-border text-xs">·</span>
          <div className="flex items-baseline gap-1">
            <span className="text-base font-bold font-mono text-text-secondary">{automatic}</span>
            <span className="text-[9px] text-text-muted">otomatik</span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Featured earthquake card (strongest in last 24h) ─────────────────────────

const NETWORK_CHIP_STYLES: Record<string, string> = {
  USGS: 'bg-blue-950/60 text-blue-400 border-blue-800/50',
  EMSC: 'bg-purple-950/60 text-purple-400 border-purple-800/50',
  AFAD: 'bg-teal-950/60 text-teal-400 border-teal-800/50',
};

function NetworkChip({ network }: { network: string }) {
  const cls =
    NETWORK_CHIP_STYLES[network.toUpperCase()] ??
    'bg-surface-elevated text-text-muted border-border';
  return (
    <span className={`text-[9px] font-bold px-1.5 py-px rounded border tracking-widest ${cls}`}>
      {network.toUpperCase()}
    </span>
  );
}

function FeaturedEarthquakeCard({ events }: { events: DisplayEvent[] }) {
  // Compute directly — no useMemo so the result is always in sync with the
  // current `events` prop (which is already filtered by the Turkey toggle).
  const cutoff = Date.now() - 24 * 3_600_000;
  const recent = events.filter(
    (e) => new Date(e.event_time).getTime() > cutoff,
  );
  const strongest =
    recent.length === 0
      ? null
      : recent.reduce((best, e) =>
          (e.ml_magnitude ?? e.magnitude) > (best.ml_magnitude ?? best.magnitude)
            ? e
            : best,
        );

  if (!strongest) return null;

  const displayMag = strongest.ml_magnitude ?? strongest.magnitude;
  const info = getMagnitudeInfo(displayMag);

  return (
    <div className="px-3 pt-2.5 pb-1 shrink-0">
      <div className="text-[9px] font-semibold uppercase tracking-widest text-text-muted mb-1.5 px-0.5">
        En Büyük Deprem (24 sa)
      </div>
      <Link
        href={`/deprem/${encodeURIComponent(strongest.source_id)}`}
        className={`block rounded border ${info.borderClass} ${info.bgClass} px-3 py-2.5 hover:brightness-110 transition-all`}
      >
        <div className="flex items-start gap-3">
          {/* Large magnitude */}
          <div className="shrink-0">
            <div className={`text-3xl font-bold font-mono leading-none ${info.textClass}`}>
              {formatMagnitude(displayMag)}
            </div>
            <div className={`text-[9px] uppercase tracking-widest mt-0.5 ${info.textClass} opacity-75`}>
              {info.label}
            </div>
          </div>
          {/* Details */}
          <div className="flex-1 min-w-0">
            <p
              className="text-sm font-semibold text-text-primary leading-tight truncate"
              title={strongest.region_name ?? strongest.source_id}
            >
              {strongest.region_name ?? strongest.source_id}
            </p>
            <div className="flex items-center gap-1.5 mt-1.5 flex-wrap">
              <span className="text-[10px] font-mono text-text-muted">
                {formatRelativeTime(strongest.event_time)}
              </span>
              <NetworkChip network={strongest.source_network} />
              {strongest.alert_level && (
                <span className="text-[9px] font-bold text-mag-red tracking-widest">
                  ⚠ {strongest.alert_level}
                </span>
              )}
            </div>
          </div>
        </div>
      </Link>
    </div>
  );
}

// ── Right panel ───────────────────────────────────────────────────────────────

const ALERT_LEVEL_STYLES: Record<string, { bg: string; text: string; border: string }> = {
  RED:    { bg: 'bg-red-950/70',    text: 'text-mag-red',    border: 'border-mag-red/40' },
  ORANGE: { bg: 'bg-orange-950/60', text: 'text-mag-orange', border: 'border-mag-orange/40' },
  YELLOW: { bg: 'bg-yellow-950/40', text: 'text-mag-yellow', border: 'border-mag-yellow/40' },
};

function PanelSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="border-b border-border last:border-0">
      <div className="px-4 py-2 border-b border-border/50 bg-surface">
        <h3 className="text-[9px] font-semibold uppercase tracking-widest text-text-muted">
          {title}
        </h3>
      </div>
      <div className="p-4">{children}</div>
    </div>
  );
}

function RecentAlertsPanel({ alerts }: { alerts: AlertEvent[] }) {
  if (alerts.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-6 select-none opacity-70">
        <svg
          width="48"
          height="48"
          viewBox="0 0 48 48"
          fill="none"
          className="text-text-muted"
        >
          <path
            d="M24 4L6 14v14c0 9.94 7.58 19.22 18 21.37C34.42 47.22 42 37.94 42 28V14L24 4z"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinejoin="round"
          />
          <path
            d="M16 24l6 6 10-10"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        <div className="text-center leading-snug">
          <p className="text-sm font-medium text-text-secondary">Uyarı yok</p>
          <p className="text-xs text-text-muted mt-0.5">Eşik değeri aşılmadı</p>
        </div>
      </div>
    );
  }

  return (
    <ul className="space-y-2">
      {alerts.slice(0, 5).map((a) => {
        const style =
          ALERT_LEVEL_STYLES[a.alert_level] ?? ALERT_LEVEL_STYLES['YELLOW'];
        return (
          <li
            key={`${a.source_id}-${a.triggered_at_ms}`}
            className={`rounded border px-3 py-2 ${style.bg} ${style.border}`}
          >
            <div className="flex items-center gap-2">
              <span
                className={`text-[9px] font-bold tracking-widest px-1 rounded border ${style.text} ${style.border}`}
              >
                {a.alert_level}
              </span>
              <span className={`text-sm font-bold font-mono ${style.text}`}>
                M {formatMagnitude(a.ml_magnitude)}
              </span>
            </div>
            <p className="text-xs text-text-secondary mt-1 truncate leading-tight">
              {a.region_name ??
                `${a.latitude.toFixed(2)}°N ${a.longitude.toFixed(2)}°E`}
            </p>
            <p className="text-[10px] text-text-muted mt-0.5">
              {formatRelativeTime(new Date(a.triggered_at_ms).toISOString())}
            </p>
          </li>
        );
      })}
    </ul>
  );
}

function RightStatsPanel({
  bands,
  alerts,
}: {
  bands: BandStats[];
  alerts: AlertEvent[];
}) {
  const { total24h, total1h, maxMag24h, rate24h } = derivedStats(bands);
  const maxInfo = getMagnitudeInfo(maxMag24h);

  return (
    <div className="flex flex-col min-h-0 overflow-y-auto">
      <PanelSection title="Gerçek Zamanlı İstatistikler">
        <dl className="space-y-3">
          <div className="flex justify-between items-baseline">
            <dt className="text-xs text-text-muted">Son 1 saat</dt>
            <dd className="text-sm font-mono font-bold text-text-primary">
              {total1h} deprem
            </dd>
          </div>
          <div className="flex justify-between items-baseline">
            <dt className="text-xs text-text-muted">Son 24 saat</dt>
            <dd className="text-sm font-mono font-bold text-text-primary">
              {total24h} deprem
            </dd>
          </div>
          <div className="flex justify-between items-baseline">
            <dt className="text-xs text-text-muted">En yüksek büyüklük</dt>
            <dd
              className={`text-sm font-mono font-bold ${
                maxMag24h > 0 ? maxInfo.textClass : 'text-text-muted'
              }`}
            >
              {maxMag24h > 0 ? `M ${formatMagnitude(maxMag24h)}` : '—'}
            </dd>
          </div>
          <div className="flex justify-between items-baseline">
            <dt className="text-xs text-text-muted">Ortalama sıklık</dt>
            <dd className="text-sm font-mono font-bold text-text-secondary">
              {rate24h.toFixed(1)} /sa
            </dd>
          </div>
        </dl>
      </PanelSection>

      <PanelSection title="Büyüklük Dağılımı (24 sa)">
        <MagnitudeChart bands={bands} />
      </PanelSection>

      <PanelSection title="Son Uyarılar">
        <RecentAlertsPanel alerts={alerts} />
      </PanelSection>
    </div>
  );
}

// ── Turkey filter toggle button ───────────────────────────────────────────────

function TurkeyFilterToggle({
  active,
  onToggle,
}: {
  active: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      onClick={onToggle}
      title={active ? 'Türkiye filtresi aktif — tümünü göster' : 'Türkiye sınırlarına filtrele'}
      className={`
        flex items-center gap-1.5 px-2.5 py-1.5 rounded text-[10px] font-bold
        tracking-widest border backdrop-blur-sm transition-all
        ${
          active
            ? 'bg-accent/20 border-accent/60 text-accent shadow-[0_0_8px_rgba(59,130,246,0.2)]'
            : 'bg-bg/80 border-border text-text-muted hover:text-text-secondary hover:border-border/80'
        }
      `}
    >
      {/* Simple map-pin icon */}
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        <circle cx="5" cy="4" r="1.5" stroke="currentColor" strokeWidth="1.2" />
        <path
          d="M5 9C5 9 1.5 6 1.5 4a3.5 3.5 0 0 1 7 0C8.5 6 5 9 5 9z"
          stroke="currentColor"
          strokeWidth="1.2"
          strokeLinejoin="round"
        />
      </svg>
      TÜRKİYE
      {active && (
        <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
      )}
    </button>
  );
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

export function Dashboard() {
  const [events, setEvents]           = useState<DisplayEvent[]>([]);
  const [bands, setBands]             = useState<BandStats[]>([]);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [alerts, setAlerts]           = useState<AlertEvent[]>([]);
  const [loading, setLoading]         = useState(true);
  const [filterTurkey, setFilterTurkey] = useState(false);

  // ── Filtered events (Turkey bbox or all) ───────────────────────────────────
  const filteredEvents = useMemo(
    () => (filterTurkey ? events.filter(inTurkey) : events),
    [events, filterTurkey],
  );

  // ── Initial REST load ─────────────────────────────────────────────────────
  const loadData = useCallback(async () => {
    try {
      const [listResp, statsResp] = await Promise.all([
        fetchEvents({ page_size: 50, page: 1 }),
        fetchStats(),
      ]);
      setEvents(listResp.events.map(apiEventToDisplay));
      setBands(statsResp.bands);
      setLastUpdated(new Date());
    } catch (err) {
      console.error('Dashboard initial load failed:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadData();
    const interval = setInterval(() => {
      fetchStats()
        .then((r) => setBands(r.bands))
        .catch(() => undefined);
    }, 60_000);
    return () => clearInterval(interval);
  }, [loadData]);

  // ── WebSocket ─────────────────────────────────────────────────────────────
  const { status: wsStatus, lastMessage } = useWebSocket(WS_URL);

  useEffect(() => {
    if (!lastMessage) return;

    if (lastMessage.type === 'earthquake') {
      const e = lastMessage as EnrichedEvent;
      const display = enrichedToDisplay(e);
      setEvents((prev) => {
        const filtered = prev.filter((p) => p.source_id !== display.source_id);
        return [display, ...filtered].slice(0, MAX_EVENTS);
      });
      setLastUpdated(new Date());
    } else if (lastMessage.type === 'alert') {
      const a = lastMessage as AlertEvent;
      setEvents((prev) =>
        prev.map((e) =>
          e.source_id === a.source_id ? { ...e, alert_level: a.alert_level } : e,
        ),
      );
      setAlerts((prev) => [a, ...prev].slice(0, MAX_ALERTS));
    }
  }, [lastMessage]);

  const risk = riskFromBands(bands);

  // ── Render ────────────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col h-screen overflow-hidden bg-bg">

      {/* ── Top header bar ── */}
      <Header wsStatus={wsStatus} lastUpdated={lastUpdated} />

      {/* ── Statistics strip — always shows global (unfiltered) band stats ── */}
      <TopStatsBar bands={bands} events={events} />

      {/* ── Three-column body ── */}
      <div className="flex flex-1 min-h-0 overflow-hidden">

        {/* ── Left sidebar ── */}
        <aside className="w-[268px] shrink-0 flex flex-col min-h-0 border-r border-border bg-surface">

          {/* Sidebar header */}
          <div className="flex items-center justify-between px-4 py-2.5 border-b border-border shrink-0">
            <div className="flex items-center gap-2">
              <span
                className="w-1.5 h-1.5 rounded-full shrink-0 animate-pulse"
                style={{ backgroundColor: risk.color }}
              />
              <h2 className="text-[9px] font-semibold uppercase tracking-widest text-text-muted">
                Son Depremler
              </h2>
              <span
                className="text-[9px] font-bold uppercase tracking-widest"
                style={{ color: risk.color }}
              >
                {risk.label}
              </span>
            </div>
            <span className="text-[9px] font-mono text-text-muted">
              {filterTurkey
                ? `${filteredEvents.length} / ${events.length}`
                : events.length > 0
                  ? events.length
                  : ''}
            </span>
          </div>

          {/* Featured strongest earthquake card */}
          <FeaturedEarthquakeCard events={filteredEvents} />

          {/* Divider between featured and list */}
          {filteredEvents.some(
            (e) => new Date(e.event_time).getTime() > Date.now() - 24 * 3_600_000,
          ) && (
            <div className="mx-3 border-t border-border/40 shrink-0" />
          )}

          {/* Scrollable event list */}
          <div className="overflow-y-auto flex-1 overscroll-contain">
            <EarthquakeList events={filteredEvents.slice(0, 50)} />
          </div>

        </aside>

        {/* ── Center: dominant map ── */}
        <main className="flex-1 min-h-0 min-w-0 relative">

          {/* Turkey filter toggle — overlaid top-left of map */}
          <div className="absolute top-3 left-3 z-10">
            <TurkeyFilterToggle
              active={filterTurkey}
              onToggle={() => setFilterTurkey((f) => !f)}
            />
          </div>

          {loading ? (
            <div className="w-full h-full bg-surface flex items-center justify-center text-text-muted text-sm">
              Yükleniyor…
            </div>
          ) : (
            <EarthquakeMap events={filteredEvents} />
          )}
        </main>

        {/* ── Right panel: stats + chart + alerts ── */}
        <aside className="w-[268px] shrink-0 flex flex-col min-h-0 border-l border-border bg-surface">
          {bands.length === 0 ? (
            <div className="flex items-center justify-center flex-1 text-text-muted text-sm">
              Yükleniyor…
            </div>
          ) : (
            <RightStatsPanel bands={bands} alerts={alerts} />
          )}
        </aside>

      </div>
    </div>
  );
}

'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import dynamic from 'next/dynamic';
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
import { formatMagnitude } from '@/lib/magnitude';

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

// ── Risk status banner ────────────────────────────────────────────────────────

interface RiskLevel {
  label: string;
  desc: string;
  dotColor: string;
  barClass: string;
  textClass: string;
}

function computeRiskLevel(bands: BandStats[]): RiskLevel {
  const m = Object.fromEntries(bands.map((b) => [b.band, b]));
  const strong = m['strong'];
  const major  = m['major'];
  const moderate = m['moderate'];
  const light  = m['light'];

  if ((strong?.count_24h ?? 0) > 0 || (major?.count_24h ?? 0) > 0) {
    const maxMag = Math.max(strong?.max_mag_24h ?? 0, major?.max_mag_24h ?? 0);
    return {
      label: 'TEHLİKE',
      desc: `Son 24 saatte M ${formatMagnitude(maxMag)} güçlü deprem kaydedildi`,
      dotColor: '#ef4444',
      barClass: 'bg-red-950/60 border-mag-red/50',
      textClass: 'text-mag-red',
    };
  }
  if ((moderate?.count_24h ?? 0) > 0) {
    return {
      label: 'UYARI',
      desc: `Son 24 saatte ${moderate.count_24h} orta büyüklüklü deprem (M 4–6)`,
      dotColor: '#f97316',
      barClass: 'bg-orange-950/40 border-mag-orange/40',
      textClass: 'text-mag-orange',
    };
  }
  if ((light?.count_24h ?? 0) > 0) {
    return {
      label: 'AKTİF',
      desc: `Son 24 saatte ${light.count_24h} hafif deprem (M 2–4)`,
      dotColor: '#eab308',
      barClass: 'bg-yellow-950/20 border-mag-yellow/30',
      textClass: 'text-mag-yellow',
    };
  }
  return {
    label: 'NORMAL',
    desc: 'Son 24 saatte kayda değer sismik aktivite yok',
    dotColor: '#22c55e',
    barClass: 'bg-green-950/20 border-mag-green/30',
    textClass: 'text-mag-green',
  };
}

function RiskBanner({ bands }: { bands: BandStats[] }) {
  if (bands.length === 0) {
    return (
      <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-surface shrink-0">
        <span className="w-2 h-2 rounded-full bg-text-muted shrink-0 animate-pulse" />
        <span className="text-xs text-text-muted">Sismik durum yükleniyor…</span>
      </div>
    );
  }

  const risk = computeRiskLevel(bands);

  // Build a compact count summary for active bands
  const BAND_SHORT: Record<string, string> = {
    minor: 'çok hafif', light: 'hafif', moderate: 'orta',
    strong: 'güçlü', major: 'büyük',
  };
  const summary = bands
    .filter((b) => b.count_24h > 0)
    .map((b) => `${b.count_24h} ${BAND_SHORT[b.band] ?? b.band}`)
    .join(' · ');

  return (
    <div className={`flex items-center gap-3 px-4 py-2 border-b shrink-0 ${risk.barClass}`}>
      <span
        className="w-2 h-2 rounded-full shrink-0"
        style={{ backgroundColor: risk.dotColor }}
      />
      <span className={`text-xs font-bold tracking-widest shrink-0 ${risk.textClass}`}>
        {risk.label}
      </span>
      <span className="text-xs text-text-secondary truncate">{risk.desc}</span>
      {summary && (
        <span className="ml-auto text-xs font-mono text-text-muted shrink-0 hidden sm:block">
          {summary}
        </span>
      )}
    </div>
  );
}

// ── Active alert banner ───────────────────────────────────────────────────────

const ALERT_BANNER_COLORS: Record<string, string> = {
  RED:    'border-mag-red bg-red-950/80 text-mag-red',
  ORANGE: 'border-mag-orange bg-orange-950/80 text-mag-orange',
  YELLOW: 'border-mag-yellow bg-yellow-950/80 text-mag-yellow',
};

function AlertBanner({ alert, onDismiss }: { alert: AlertEvent; onDismiss: () => void }) {
  const cls = ALERT_BANNER_COLORS[alert.alert_level] ?? ALERT_BANNER_COLORS['YELLOW'];
  return (
    <div className={`border-b px-4 py-2.5 flex items-center gap-3 shrink-0 ${cls}`}>
      <span className="font-bold text-sm tracking-wide">⚠ DEPREM UYARISI</span>
      <span className="text-sm truncate">
        M {formatMagnitude(alert.ml_magnitude)} —{' '}
        {alert.region_name ?? `${alert.latitude.toFixed(2)}°N, ${alert.longitude.toFixed(2)}°E`}
      </span>
      <span className="ml-auto shrink-0 px-2 py-0.5 rounded text-xs font-bold border border-current">
        {alert.alert_level}
      </span>
      <button
        onClick={onDismiss}
        className="text-current opacity-60 hover:opacity-100 shrink-0 font-mono"
        aria-label="Kapat"
      >
        ✕
      </button>
    </div>
  );
}

// ── Section header ────────────────────────────────────────────────────────────

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="text-xs font-semibold uppercase tracking-widest text-text-muted px-4 py-2 border-b border-border bg-surface shrink-0">
      {children}
    </h2>
  );
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

export function Dashboard() {
  const [events, setEvents]           = useState<DisplayEvent[]>([]);
  const [bands, setBands]             = useState<BandStats[]>([]);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [activeAlert, setActiveAlert] = useState<AlertEvent | null>(null);
  const [loading, setLoading]         = useState(true);

  const alertTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Initial data load ─────────────────────────────────────────────────────
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
      // Tag the matching event in the list with the alert level
      setEvents((prev) =>
        prev.map((e) =>
          e.source_id === a.source_id ? { ...e, alert_level: a.alert_level } : e,
        ),
      );
      setActiveAlert(a);
      if (alertTimerRef.current) clearTimeout(alertTimerRef.current);
      alertTimerRef.current = setTimeout(() => setActiveAlert(null), 60_000);
    }
  }, [lastMessage]);

  useEffect(() => {
    return () => {
      if (alertTimerRef.current) clearTimeout(alertTimerRef.current);
    };
  }, []);

  // ── Render ────────────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col h-screen overflow-hidden">
      <Header wsStatus={wsStatus} lastUpdated={lastUpdated} />

      {/* Persistent seismic risk status */}
      <RiskBanner bands={bands} />

      {/* Dismissible alert banner (fires on WebSocket alert events) */}
      {activeAlert && (
        <AlertBanner alert={activeAlert} onDismiss={() => setActiveAlert(null)} />
      )}

      {/* ── Map — dominant, full width ─────────────────────────────────── */}
      <main className="flex-1 min-h-0 relative">
        {loading ? (
          <div className="w-full h-full bg-surface flex items-center justify-center text-text-muted text-sm">
            Yükleniyor…
          </div>
        ) : (
          <EarthquakeMap events={events} />
        )}
      </main>

      {/* ── Compact bottom panel ───────────────────────────────────────── */}
      <div className="h-64 xl:h-72 shrink-0 border-t border-border flex flex-row overflow-hidden">
        {/* Recent earthquake list */}
        <div className="flex-[3] flex flex-col min-h-0 border-r border-border">
          <SectionTitle>
            Son Depremler
            {events.length > 0 && (
              <span className="ml-2 text-text-muted font-normal normal-case tracking-normal">
                ({events.length})
              </span>
            )}
          </SectionTitle>
          <div className="overflow-y-auto flex-1">
            <EarthquakeList events={events.slice(0, 50)} />
          </div>
        </div>

        {/* Magnitude distribution chart */}
        <div className="flex-[2] flex flex-col min-h-0">
          <SectionTitle>Büyüklük Dağılımı (24 sa)</SectionTitle>
          <div className="p-4 overflow-auto flex-1">
            {bands.length > 0 ? (
              <MagnitudeChart bands={bands} />
            ) : (
              <div className="text-text-muted text-sm">
                {loading ? 'Yükleniyor…' : 'Veri yok'}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

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
import { MagnitudeChart, RegionalStats } from '@/components/RegionalStats';
import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

// Dynamically import the map (browser-only, requires maplibre-gl)
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

// ── Alert banner ──────────────────────────────────────────────────────────────

function AlertBanner({ alert, onDismiss }: { alert: AlertEvent; onDismiss: () => void }) {
  const info = getMagnitudeInfo(alert.ml_magnitude);
  const levelColors: Record<string, string> = {
    RED: 'border-mag-red bg-red-950/80 text-mag-red',
    ORANGE: 'border-mag-orange bg-orange-950/80 text-mag-orange',
    YELLOW: 'border-mag-yellow bg-yellow-950/80 text-mag-yellow',
  };
  const cls = levelColors[alert.alert_level] ?? levelColors['YELLOW'];

  return (
    <div className={`border rounded px-4 py-2.5 flex items-center gap-3 mb-3 ${cls}`}>
      <span className="font-bold text-sm tracking-wide">⚠ DEPREM UYARISI</span>
      <span className="text-sm">
        M {formatMagnitude(alert.ml_magnitude)} —{' '}
        {alert.region_name ?? `${alert.latitude.toFixed(2)}°N, ${alert.longitude.toFixed(2)}°E`}
      </span>
      <span className={`ml-auto px-2 py-0.5 rounded text-xs font-bold border ${info.borderClass}`}>
        {alert.alert_level}
      </span>
      <button
        onClick={onDismiss}
        className="text-current opacity-60 hover:opacity-100 ml-1 font-mono"
        aria-label="Kapat"
      >
        ✕
      </button>
    </div>
  );
}

// ── Section header helper ─────────────────────────────────────────────────────

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="text-xs font-semibold uppercase tracking-widest text-text-muted px-4 py-2 border-b border-border bg-surface shrink-0">
      {children}
    </h2>
  );
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

export function Dashboard() {
  const [events, setEvents] = useState<DisplayEvent[]>([]);
  const [bands, setBands] = useState<BandStats[]>([]);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [activeAlert, setActiveAlert] = useState<AlertEvent | null>(null);
  const [loading, setLoading] = useState(true);

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
    // Refresh stats every 60 s
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
        // Deduplicate by source_id; live event overrides historical
        const filtered = prev.filter((p) => p.source_id !== display.source_id);
        return [display, ...filtered].slice(0, MAX_EVENTS);
      });
      setLastUpdated(new Date());
    } else if (lastMessage.type === 'alert') {
      const a = lastMessage as AlertEvent;
      setActiveAlert(a);
      // Auto-dismiss alert after 60 s
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

      {/* Alert banner */}
      {activeAlert && (
        <div className="px-4 pt-3 shrink-0">
          <AlertBanner alert={activeAlert} onDismiss={() => setActiveAlert(null)} />
        </div>
      )}

      {/* Main grid: map + sidebar */}
      <div className="flex flex-1 overflow-hidden">
        {/* ── Map ─────────────────────────────────────────────────── */}
        <main className="flex-1 relative min-h-0">
          {loading ? (
            <div className="w-full h-full bg-surface flex items-center justify-center text-text-muted text-sm">
              Yükleniyor…
            </div>
          ) : (
            <EarthquakeMap events={events} />
          )}
        </main>

        {/* ── Sidebar ─────────────────────────────────────────────── */}
        <aside className="w-80 xl:w-96 border-l border-border bg-surface flex flex-col overflow-hidden shrink-0">
          {/* Recent earthquakes list */}
          <div className="flex flex-col min-h-0 flex-[3]">
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
          <div className="border-t border-border flex-[2] flex flex-col min-h-0">
            <SectionTitle>Büyüklük Dağılımı (24 sa)</SectionTitle>
            <div className="p-4 flex-1 overflow-auto">
              {bands.length > 0 ? (
                <MagnitudeChart bands={bands} />
              ) : (
                <div className="text-text-muted text-sm">
                  İstatistik verisi yok
                </div>
              )}
            </div>
          </div>
        </aside>
      </div>

      {/* ── Regional stats bar ──────────────────────────────────────── */}
      <footer className="border-t border-border bg-surface p-4 shrink-0">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-text-muted">
            Büyüklük Bandı İstatistikleri
          </h2>
        </div>
        {bands.length > 0 ? (
          <RegionalStats bands={bands} />
        ) : (
          <div className="text-text-muted text-sm">
            {loading ? 'Yükleniyor…' : 'Veri yok'}
          </div>
        )}
      </footer>
    </div>
  );
}

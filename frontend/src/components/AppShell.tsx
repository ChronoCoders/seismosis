'use client';

import { useState, useEffect, useCallback } from 'react';
import type { DisplayEvent, AlertEvent, EnrichedEvent, BandStats } from '@/types';
import { apiEventToDisplay, enrichedToDisplay } from '@/types';
import { fetchEvents, fetchStats } from '@/lib/api';
import { useWebSocket } from '@/hooks/useWebSocket';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { Sidebar } from '@/components/Sidebar';
import { LiveMap } from '@/components/pages/LiveMap';
import { AlertsPage } from '@/components/pages/AlertsPage';
import { RegionalPage } from '@/components/pages/RegionalPage';
import { StatsPage } from '@/components/pages/StatsPage';
import { DataSourcesPage } from '@/components/pages/DataSourcesPage';
import { SettingsPage } from '@/components/pages/SettingsPage';
import RealtimeTab from '@/components/tabs/RealtimeTab';
import HistoryTab from '@/components/tabs/HistoryTab';
import CompareTab from '@/components/tabs/CompareTab';

export type PageId =
  | 'live-map'
  | 'realtime'
  | 'alerts'
  | 'history'
  | 'regional'
  | 'compare'
  | 'stats'
  | 'sources'
  | 'settings';

const WS_URL =
  typeof window !== 'undefined'
    ? (process.env.NEXT_PUBLIC_WS_URL ?? 'ws://localhost:9093')
    : '';

const MAX_EVENTS = 200;
const MAX_ALERTS = 50;

export function AppShell() {
  const [activePage, setActivePage]     = useState<PageId>('live-map');
  const [sidebarExpanded, setSidebarExpanded] = useState(true);
  const [events, setEvents]             = useState<DisplayEvent[]>([]);
  const [bands, setBands]               = useState<BandStats[]>([]);
  const [alerts, setAlerts]             = useState<AlertEvent[]>([]);
  const [lastUpdated, setLastUpdated]   = useState<Date | null>(null);
  const [loading, setLoading]           = useState(true);
  const [filterTurkey, setFilterTurkey] = useState(false);
  const [minMagnitude, setMinMagnitude] = useState(0.0);

  // ── Data loading ────────────────────────────────────────────────────────────

  const loadData = useCallback(async () => {
    try {
      const [listResp, statsResp] = await Promise.all([
        fetchEvents({ page_size: 100 }),
        fetchStats(),
      ]);
      setEvents(listResp.events.map(apiEventToDisplay));
      setBands(statsResp.bands);
      setLastUpdated(new Date());
    } catch (err) {
      console.error('AppShell initial load failed:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadData();
    const id = setInterval(() => {
      fetchStats().then((r) => setBands(r.bands)).catch(() => undefined);
    }, 60_000);
    return () => clearInterval(id);
  }, [loadData]);

  // ── WebSocket ────────────────────────────────────────────────────────────────

  const { status: wsStatus, lastMessage, send } = useWebSocket(WS_URL);

  useEffect(() => {
    if (wsStatus !== 'connected') return;
    send(JSON.stringify({ type: 'subscribe', min_magnitude: minMagnitude }));
  }, [wsStatus, minMagnitude, send]);

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

  // ── Derived state ────────────────────────────────────────────────────────────

  const unreadAlerts = alerts.length;

  // ── Page renderer ────────────────────────────────────────────────────────────

  function renderPage() {
    switch (activePage) {
      case 'live-map':
        return (
          <LiveMap
            events={events}
            bands={bands}
            alerts={alerts}
            filterTurkey={filterTurkey}
            minMagnitude={minMagnitude}
            onFilterTurkey={() => setFilterTurkey((f) => !f)}
            onMinMagnitude={setMinMagnitude}
            loading={loading}
          />
        );
      case 'realtime':
        return <RealtimeTab events={events} wsStatus={wsStatus} />;
      case 'alerts':
        return <AlertsPage alerts={alerts} />;
      case 'history':
        return <HistoryTab />;
      case 'regional':
        return <RegionalPage events={events} />;
      case 'compare':
        return <CompareTab />;
      case 'stats':
        return <StatsPage bands={bands} events={events} />;
      case 'sources':
        return <DataSourcesPage events={events} />;
      case 'settings':
        return (
          <SettingsPage
            minMagnitude={minMagnitude}
            onMinMagnitude={setMinMagnitude}
            filterTurkey={filterTurkey}
            onFilterTurkey={() => setFilterTurkey((f) => !f)}
          />
        );
    }
  }

  return (
    <div className="flex h-screen overflow-hidden bg-[#0d0f14]">
      <Sidebar
        activePage={activePage}
        onNavigate={setActivePage}
        expanded={sidebarExpanded}
        onToggle={() => setSidebarExpanded((e) => !e)}
        wsStatus={wsStatus}
        lastUpdated={lastUpdated}
        alertCount={unreadAlerts}
      />
      <main className="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden">
        <ErrorBoundary>
          {renderPage()}
        </ErrorBoundary>
      </main>
    </div>
  );
}

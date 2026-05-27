'use client';

import { useMemo } from 'react';
import { CheckCircle, AlertCircle, Clock } from 'lucide-react';
import type { DisplayEvent } from '@/types';

const SOURCES = [
  {
    id: 'USGS',
    name: 'USGS',
    full: 'U.S. Geological Survey',
    description: 'ABD merkezli küresel deprem ağı, FDSN standartlarında veri',
    color: '#60a5fa',
    bg: 'bg-blue-950/30',
    border: 'border-blue-700/40',
  },
  {
    id: 'EMSC',
    name: 'EMSC',
    full: 'European-Mediterranean Seismological Centre',
    description: 'Avrupa ve Akdeniz bölgesi odaklı sismik izleme',
    color: '#c084fc',
    bg: 'bg-purple-950/30',
    border: 'border-purple-700/40',
  },
  {
    id: 'AFAD',
    name: 'AFAD',
    full: 'Afet ve Acil Durum Yönetimi Başkanlığı',
    description: 'Türkiye ulusal deprem izleme ağı, ~2.5 saat gecikmeyle yayımlanır',
    color: '#2dd4bf',
    bg: 'bg-teal-950/30',
    border: 'border-teal-700/40',
  },
];

function relativeAgo(ms: number): string {
  const diff = Date.now() - ms;
  const s = Math.floor(diff / 1000);
  if (s < 60)    return `${s}s önce`;
  const m = Math.floor(s / 60);
  if (m < 60)    return `${m}dk önce`;
  const h = Math.floor(m / 60);
  if (h < 24)    return `${h}sa önce`;
  return `${Math.floor(h / 24)}g önce`;
}

interface SourceStats {
  count: number;
  lastEventMs: number | null;
  count24h: number;
}

interface Props {
  events: DisplayEvent[];
}

export function DataSourcesPage({ events }: Props) {
  const statsMap = useMemo(() => {
    const map: Record<string, SourceStats> = {};
    const cutoff24h = Date.now() - 86_400_000;

    for (const e of events) {
      const src = e.source_network.toUpperCase();
      if (!map[src]) map[src] = { count: 0, lastEventMs: null, count24h: 0 };
      const t = new Date(e.event_time).getTime();
      map[src].count++;
      if (!map[src].lastEventMs || t > map[src].lastEventMs!) map[src].lastEventMs = t;
      if (t >= cutoff24h) map[src].count24h++;
    }
    return map;
  }, [events]);

  function healthStatus(stats: SourceStats | undefined, srcId: string): 'healthy' | 'stale' | 'unknown' {
    if (!stats || stats.lastEventMs === null) return 'unknown';
    // AFAD has ~2.5hr publication delay; use a 6h staleness threshold for it
    const threshold = srcId === 'AFAD' ? 6 * 3_600_000 : 3_600_000;
    return Date.now() - stats.lastEventMs < threshold ? 'healthy' : 'stale';
  }

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      <div className="flex items-center px-5 h-12 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div>
          <h1 className="text-sm font-semibold text-[#e2e4ed]">Veri Kaynakları</h1>
          <p className="text-[10px] text-[#575c6e] mt-0.5">
            Ingestion servisi bağlantı durumu ve olay istatistikleri
          </p>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-5">
        <div className="max-w-3xl space-y-4">
          {SOURCES.map((src) => {
            const stats = statsMap[src.id];
            const health = healthStatus(stats, src.id);

            return (
              <div key={src.id} className={`rounded-lg border ${src.border} ${src.bg} p-5`}>
                <div className="flex items-start gap-4">
                  {/* Status icon */}
                  <div className="shrink-0 mt-0.5">
                    {health === 'healthy' ? (
                      <CheckCircle size={18} className="text-emerald-400" />
                    ) : health === 'stale' ? (
                      <Clock size={18} className="text-yellow-400" />
                    ) : (
                      <AlertCircle size={18} className="text-[#575c6e]" />
                    )}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-3 flex-wrap">
                      <span className="text-sm font-bold" style={{ color: src.color }}>{src.name}</span>
                      <span className="text-xs text-[#8b90a2]">{src.full}</span>
                      <span className={`ml-auto text-[9px] font-bold tracking-widest px-2 py-0.5 rounded ${
                        health === 'healthy'
                          ? 'bg-emerald-900/60 text-emerald-400'
                          : health === 'stale'
                          ? 'bg-yellow-900/50 text-yellow-400'
                          : 'bg-[#1a1e2b] text-[#575c6e]'
                      }`}>
                        {health === 'healthy' ? 'SAĞLIKLI' : health === 'stale' ? 'GECİKMELİ' : 'BİLİNMİYOR'}
                      </span>
                    </div>
                    <p className="text-[10px] text-[#575c6e] mt-1">{src.description}</p>

                    <div className="mt-4 grid grid-cols-3 gap-4">
                      <div>
                        <p className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">Bellekteki Olay</p>
                        <p className="text-xl font-bold font-mono text-[#e2e4ed] mt-1">
                          {stats?.count ?? 0}
                        </p>
                      </div>
                      <div>
                        <p className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">Son 24 Saat</p>
                        <p className="text-xl font-bold font-mono text-[#e2e4ed] mt-1">
                          {stats?.count24h ?? 0}
                        </p>
                      </div>
                      <div>
                        <p className="text-[9px] font-semibold uppercase tracking-widest text-[#575c6e]">Son Olay</p>
                        <p className="text-sm font-mono text-[#8b90a2] mt-1">
                          {stats?.lastEventMs ? relativeAgo(stats.lastEventMs) : '—'}
                        </p>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            );
          })}

          {/* Info note */}
          <div className="rounded border border-[#232736] bg-[#12151d] px-4 py-3">
            <p className="text-[10px] text-[#575c6e] leading-relaxed">
              <span className="font-semibold text-[#8b90a2]">Not:</span>{' '}
              İstatistikler, mevcut oturumdaki bellek içi olay listesinden hesaplanmaktadır.
              Ingestion servisinin Prometheus metrikleri için{' '}
              <span className="font-mono text-[#4a90e2]">:9091/metrics</span> adresini kullanın.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

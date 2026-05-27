'use client';

import { useState, useEffect } from 'react';
import { Shield } from 'lucide-react';
import type { AlertEvent, EarthquakeEvent } from '@/types';
import { fetchEvents } from '@/lib/api';
import { getMagnitudeInfo, formatMagnitude, formatRelativeTime } from '@/lib/magnitude';

const SEVERITY: Record<string, { bg: string; border: string; text: string; badge: string }> = {
  RED:    { bg: 'bg-red-950/40',    border: 'border-red-700/50',    text: 'text-red-400',    badge: 'bg-red-900/60 text-red-300 border-red-700/60'    },
  ORANGE: { bg: 'bg-orange-950/40', border: 'border-orange-700/50', text: 'text-orange-400', badge: 'bg-orange-900/60 text-orange-300 border-orange-700/60' },
  YELLOW: { bg: 'bg-yellow-950/30', border: 'border-yellow-700/40', text: 'text-yellow-400', badge: 'bg-yellow-900/50 text-yellow-300 border-yellow-700/50' },
};

interface Props {
  alerts: AlertEvent[];
}

export function AlertsPage({ alerts }: Props) {
  const [significant, setSignificant] = useState<EarthquakeEvent[]>([]);

  useEffect(() => {
    const start = new Date(Date.now() - 86_400_000).toISOString();
    fetchEvents({ min_magnitude: 4.0, start_time: start, page_size: 100 })
      .then((r) => setSignificant(r.events))
      .catch(() => undefined);
  }, []);

  const isEmpty = alerts.length === 0 && significant.length === 0;

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      <div className="flex items-center justify-between px-5 h-12 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div>
          <h1 className="text-sm font-semibold text-[#e2e4ed]">Uyarılar</h1>
          <p className="text-[10px] text-[#575c6e] mt-0.5">Eşik değerini aşan olaylar</p>
        </div>
        <span className="text-xs font-mono text-[#575c6e]">{alerts.length} uyarı</span>
      </div>

      <div className="flex-1 overflow-y-auto p-5">
        {isEmpty ? (
          <div className="flex flex-col items-center justify-center h-full gap-4 select-none opacity-60">
            <Shield size={56} strokeWidth={1} className="text-[#575c6e]" />
            <div className="text-center">
              <p className="text-sm font-medium text-[#8b90a2]">Aktif uyarı yok</p>
              <p className="text-xs text-[#575c6e] mt-1">Son 24 saatte M≥4.0 deprem kaydedilmedi</p>
            </div>
          </div>
        ) : (
          <div className="max-w-2xl space-y-5">
            {/* Alert cards from WebSocket */}
            {alerts.length > 0 && (
              <div className="space-y-3">
                {alerts.map((alert) => {
                  const s = SEVERITY[alert.alert_level] ?? SEVERITY['YELLOW'];
                  return (
                    <div
                      key={`${alert.source_id}-${alert.triggered_at_ms}`}
                      className={`rounded-lg border ${s.bg} ${s.border} px-5 py-4`}
                    >
                      <div className="flex items-start gap-4">
                        <div className="shrink-0">
                          <span className={`inline-block text-[9px] font-bold tracking-widest px-2 py-0.5 rounded border ${s.badge}`}>
                            {alert.alert_level}
                          </span>
                          <div className={`text-2xl font-bold font-mono mt-2 ${s.text}`}>
                            M {formatMagnitude(alert.ml_magnitude)}
                          </div>
                          <div className="text-[10px] text-[#575c6e] font-mono mt-0.5">
                            {formatMagnitude(alert.magnitude)} ham
                          </div>
                        </div>
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-semibold text-[#e2e4ed] truncate">
                            {alert.region_name ?? `${alert.latitude.toFixed(2)}°N ${alert.longitude.toFixed(2)}°E`}
                          </p>
                          <div className="mt-2 grid grid-cols-2 gap-x-6 gap-y-1.5 text-[10px]">
                            <span className="text-[#575c6e]">
                              Derinlik:{' '}
                              <span className="text-[#8b90a2] font-mono">
                                {alert.depth_km != null ? `${alert.depth_km.toFixed(0)} km` : '—'}
                              </span>
                            </span>
                            <span className="text-[#575c6e]">
                              Yoğunluk:{' '}
                              <span className="text-[#8b90a2] font-mono">MMI {alert.estimated_intensity_mmi.toFixed(1)}</span>
                            </span>
                            <span className="text-[#575c6e]">
                              Hissedilme:{' '}
                              <span className="text-[#8b90a2] font-mono">{alert.estimated_felt_radius_km.toFixed(0)} km</span>
                            </span>
                            <span className="text-[#575c6e]">
                              {alert.is_aftershock ? (
                                <span className="text-orange-400 font-semibold">Artçı sarsıntı</span>
                              ) : (
                                <span className="text-[#8b90a2]">Bağımsız olay</span>
                              )}
                            </span>
                          </div>
                          <p className="text-[10px] font-mono text-[#575c6e] mt-2">
                            {formatRelativeTime(new Date(alert.triggered_at_ms).toISOString())}
                          </p>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            {/* Significant events from API (M≥4.0, last 24h) */}
            {significant.length > 0 && (
              <div>
                <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-3">
                  Son 24 Saatte Önemli Depremler (M≥4.0)
                </p>
                <div className="space-y-2">
                  {significant.map((ev) => {
                    const info = getMagnitudeInfo(ev.magnitude);
                    return (
                      <div
                        key={ev.source_id}
                        className="rounded border border-[#232736] bg-[#12151d] px-4 py-3 flex items-center gap-4"
                      >
                        <div className="shrink-0 w-14 text-center">
                          <p className={`text-xl font-bold font-mono leading-none ${info.textClass}`}>
                            M {formatMagnitude(ev.magnitude)}
                          </p>
                          <p className="text-[9px] text-[#575c6e] font-mono mt-0.5">{ev.magnitude_type}</p>
                        </div>
                        <div className="flex-1 min-w-0">
                          <p className="text-xs font-semibold text-[#e2e4ed] truncate">
                            {ev.region_name ?? `${ev.latitude.toFixed(2)}°N ${ev.longitude.toFixed(2)}°E`}
                          </p>
                          <div className="flex items-center gap-3 mt-1 text-[10px] font-mono text-[#575c6e]">
                            {ev.depth_km != null && <span>{ev.depth_km.toFixed(0)} km derinlik</span>}
                            <span className="uppercase tracking-widest text-[9px]">{ev.source_network}</span>
                          </div>
                        </div>
                        <p className="text-[10px] font-mono text-[#575c6e] shrink-0">
                          {formatRelativeTime(ev.event_time)}
                        </p>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

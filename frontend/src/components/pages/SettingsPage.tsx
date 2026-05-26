'use client';

interface Props {
  minMagnitude: number;
  onMinMagnitude: (v: number) => void;
  filterTurkey: boolean;
  onFilterTurkey: () => void;
}

export function SettingsPage({ minMagnitude, onMinMagnitude, filterTurkey, onFilterTurkey }: Props) {
  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      <div className="flex items-center px-5 py-3 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div>
          <h1 className="text-sm font-semibold text-[#e2e4ed]">Ayarlar</h1>
          <p className="text-[10px] text-[#575c6e] mt-0.5">Görüntüleme tercihleri</p>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-5">
        <div className="max-w-md space-y-6">
          {/* Filter defaults */}
          <div className="rounded border border-[#232736] bg-[#12151d] p-5">
            <h2 className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-4">
              Filtre Varsayılanları
            </h2>
            <div className="space-y-5">
              {/* Min magnitude */}
              <div>
                <label className="block text-xs font-semibold text-[#8b90a2] mb-2">
                  Minimum Büyüklük
                </label>
                <div className="flex items-center gap-4">
                  <input
                    type="range" min={0} max={7} step={0.5} value={minMagnitude}
                    onChange={(e) => onMinMagnitude(parseFloat(e.target.value))}
                    className="flex-1 h-1 accent-[#4a90e2] cursor-pointer"
                  />
                  <span className="text-sm font-bold font-mono text-[#e2e4ed] w-8 text-right tabular-nums">
                    M {minMagnitude.toFixed(1)}
                  </span>
                </div>
                <p className="text-[10px] text-[#575c6e] mt-1.5">
                  Bu değerin altındaki depremler harita ve listelerden gizlenir
                </p>
              </div>

              {/* Turkey filter */}
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-xs font-semibold text-[#8b90a2]">Türkiye Filtresi</p>
                  <p className="text-[10px] text-[#575c6e] mt-0.5">
                    Yalnızca Türkiye sınırları içindeki depremleri göster
                  </p>
                </div>
                <button
                  onClick={onFilterTurkey}
                  className={`relative w-10 h-5 rounded-full transition-colors ${
                    filterTurkey ? 'bg-[#4a90e2]' : 'bg-[#232736]'
                  }`}
                >
                  <span
                    className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                      filterTurkey ? 'translate-x-5' : 'translate-x-0.5'
                    }`}
                  />
                </button>
              </div>
            </div>
          </div>

          {/* System info */}
          <div className="rounded border border-[#232736] bg-[#12151d] p-5">
            <h2 className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-4">
              Sistem Bilgisi
            </h2>
            <dl className="space-y-2.5 text-xs">
              {[
                ['Platform', 'Seismosis v2'],
                ['WebSocket', process.env.NEXT_PUBLIC_WS_URL ?? 'ws://localhost:9093'],
                ['Veri Kaynakları', 'USGS · EMSC · AFAD'],
                ['Analiz', 'ML Büyüklük Kalibrasyonu · Artçı Tespiti'],
              ].map(([k, v]) => (
                <div key={k} className="flex items-start gap-3">
                  <dt className="text-[#575c6e] shrink-0 w-28">{k}</dt>
                  <dd className="text-[#8b90a2] font-mono break-all">{v}</dd>
                </div>
              ))}
            </dl>
          </div>
        </div>
      </div>
    </div>
  );
}

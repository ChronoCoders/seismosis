'use client';

import {
  Map,
  Activity,
  Bell,
  Clock,
  Globe,
  BarChart2,
  TrendingUp,
  LineChart,
  Database,
  Settings,
  ChevronLeft,
  ChevronRight,
} from 'lucide-react';
import type { PageId } from './AppShell';
import type { WsStatus } from '@/hooks/useWebSocket';

const WS_DOT: Record<WsStatus, string> = {
  connected:    'bg-emerald-400',
  connecting:   'bg-yellow-400 animate-pulse',
  disconnected: 'bg-zinc-500',
  error:        'bg-red-500',
};

const WS_LABEL: Record<WsStatus, string> = {
  connected:    'CANLI',
  connecting:   'BAĞLANIYOR',
  disconnected: 'BAĞLANTI YOK',
  error:        'HATA',
};

interface NavItem {
  id: PageId;
  label: string;
  icon: React.ElementType;
}

interface Section {
  title: string;
  items: NavItem[];
}

const SECTIONS: Section[] = [
  {
    title: 'İZLEME',
    items: [
      { id: 'live-map',  label: 'Canlı Harita',  icon: Map      },
      { id: 'realtime',  label: 'Gerçek Zamanlı', icon: Activity },
      { id: 'alerts',    label: 'Uyarılar',       icon: Bell     },
    ],
  },
  {
    title: 'ANALİZ',
    items: [
      { id: 'history',   label: 'Geçmiş',         icon: Clock     },
      { id: 'regional',  label: 'Bölgesel Analiz', icon: Globe     },
      { id: 'compare',   label: 'Karşılaştırma',   icon: BarChart2 },
      { id: 'stats',     label: 'İstatistikler',   icon: TrendingUp},
      { id: 'forecast',  label: 'Tahmin',          icon: LineChart },
    ],
  },
  {
    title: 'SİSTEM',
    items: [
      { id: 'sources',  label: 'Veri Kaynakları', icon: Database },
      { id: 'settings', label: 'Ayarlar',         icon: Settings },
    ],
  },
];

interface Props {
  activePage: PageId;
  onNavigate: (id: PageId) => void;
  expanded: boolean;
  onToggle: () => void;
  wsStatus: WsStatus;
  lastUpdated: Date | null;
  alertCount: number;
}

export function Sidebar({ activePage, onNavigate, expanded, onToggle, wsStatus, lastUpdated, alertCount }: Props) {
  const w = expanded ? 'w-[220px]' : 'w-16';

  return (
    <aside
      className={`${w} shrink-0 flex flex-col h-screen bg-[#0c0e16] border-r border-[#232736] transition-[width] duration-200 ease-in-out overflow-hidden z-20`}
    >
      {/* Logo + toggle */}
      <div className="flex items-center justify-between h-12 border-b border-[#232736] shrink-0 px-4">
        {expanded ? (
          <img
            src="/logo.svg"
            alt="seismosio"
            className="h-7 w-auto"
            draggable={false}
          />
        ) : (
          <svg
            width="24"
            height="24"
            viewBox="16 36 128 128"
            aria-label="seismosio"
          >
            <circle cx="80" cy="100" r="60" fill="none" stroke="#e05c2c" strokeWidth="8"/>
            <circle cx="80" cy="100" r="42" fill="none" stroke="#e05c2c" strokeWidth="8"/>
            <circle cx="80" cy="100" r="24" fill="none" stroke="#e05c2c" strokeWidth="8"/>
            <circle cx="80" cy="100" r="8"  fill="#e05c2c"/>
          </svg>
        )}
        <button
          onClick={onToggle}
          className="shrink-0 p-1.5 rounded text-[#575c6e] hover:text-[#e2e4ed] hover:bg-[#1a1e2b] transition-colors"
          title={expanded ? 'Küçült' : 'Genişlet'}
        >
          {expanded ? <ChevronLeft size={14} /> : <ChevronRight size={14} />}
        </button>
      </div>

      {/* Nav sections */}
      <nav className="flex-1 overflow-y-auto py-3 space-y-1">
        {SECTIONS.map((section) => (
          <div key={section.title} className="mb-1">
            {expanded && (
              <p className="px-4 pt-3 pb-1.5 text-[9px] font-bold tracking-[0.15em] text-[#575c6e] uppercase whitespace-nowrap">
                {section.title}
              </p>
            )}
            {!expanded && <div className="h-px mx-3 bg-[#232736] my-2" />}
            {section.items.map((item) => {
              const Icon = item.icon;
              const active = activePage === item.id;
              const hasBadge = item.id === 'alerts' && alertCount > 0;
              return (
                <button
                  key={item.id}
                  onClick={() => onNavigate(item.id)}
                  title={!expanded ? item.label : undefined}
                  className={`
                    w-full flex items-center gap-3 px-3 py-2.5 mx-1.5 rounded
                    text-left transition-colors text-xs font-medium whitespace-nowrap
                    ${active
                      ? 'bg-[#1a2235] text-[#e2e4ed]'
                      : 'text-[#8b90a2] hover:text-[#e2e4ed] hover:bg-[#151822]'
                    }
                  `}
                  style={{ width: expanded ? 'calc(100% - 12px)' : '40px' }}
                >
                  <span className="relative shrink-0">
                    <Icon
                      size={15}
                      strokeWidth={active ? 2 : 1.5}
                      className={active ? 'text-[#4a90e2]' : ''}
                    />
                    {hasBadge && (
                      <span className="absolute -top-1 -right-1 w-1.5 h-1.5 rounded-full bg-red-500" />
                    )}
                  </span>
                  {expanded && <span className="truncate">{item.label}</span>}
                  {expanded && active && (
                    <span className="ml-auto w-1 h-4 rounded-full bg-[#4a90e2] shrink-0" />
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </nav>

      {/* Bottom status */}
      <div className="border-t border-[#232736] shrink-0 px-3 py-3 space-y-2">
        {/* WS status */}
        <div className="flex items-center gap-2">
          <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${WS_DOT[wsStatus]}`} />
          {expanded && (
            <span className="text-[10px] font-bold tracking-widest text-[#575c6e] truncate">
              {WS_LABEL[wsStatus]}
            </span>
          )}
        </div>
        {/* Time */}
        {expanded && lastUpdated && (
          <p className="text-[9px] font-mono text-[#575c6e] truncate">
            {lastUpdated.toLocaleTimeString('tr-TR', { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
          </p>
        )}
      </div>
    </aside>
  );
}

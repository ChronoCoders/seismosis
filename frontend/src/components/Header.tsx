'use client';

import Link from 'next/link';
import type { WsStatus } from '@/hooks/useWebSocket';

interface HeaderProps {
  wsStatus: WsStatus;
  lastUpdated: Date | null;
}

const STATUS_CONFIG: Record<
  WsStatus,
  { label: string; dotClass: string; textClass: string }
> = {
  connected: {
    label: 'CANLI',
    dotClass: 'bg-mag-green animate-pulse',
    textClass: 'text-mag-green',
  },
  connecting: {
    label: 'BAĞLANIYOR',
    dotClass: 'bg-mag-yellow',
    textClass: 'text-mag-yellow',
  },
  disconnected: {
    label: 'BAĞLANTI YOK',
    dotClass: 'bg-text-muted',
    textClass: 'text-text-muted',
  },
  error: {
    label: 'HATA',
    dotClass: 'bg-mag-red',
    textClass: 'text-mag-red',
  },
};

export function Header({ wsStatus, lastUpdated }: HeaderProps) {
  const cfg = STATUS_CONFIG[wsStatus];

  return (
    <header className="h-14 bg-surface border-b border-border flex items-center px-4 shrink-0 z-20">
      <Link href="/" className="flex items-center gap-2 mr-auto">
        {/* Simple seismogram icon */}
        <svg
          width="22"
          height="22"
          viewBox="0 0 22 22"
          fill="none"
          className="text-accent"
        >
          <polyline
            points="1,11 5,11 6,7 8,15 10,5 12,17 14,9 16,13 17,11 21,11"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinejoin="round"
            strokeLinecap="round"
            fill="none"
          />
        </svg>
        <span className="font-semibold tracking-wide text-text-primary">
          SEISMOSIS
        </span>
        <span className="hidden sm:block text-text-muted text-xs ml-1">
          Gerçek Zamanlı Deprem İzleme
        </span>
      </Link>

      <div className="flex items-center gap-4">
        {lastUpdated && (
          <span className="hidden md:block text-text-muted text-xs font-mono">
            {lastUpdated.toLocaleTimeString('tr-TR')}
          </span>
        )}
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full ${cfg.dotClass}`} />
          <span className={`text-xs font-semibold tracking-widest ${cfg.textClass}`}>
            {cfg.label}
          </span>
        </div>
      </div>
    </header>
  );
}

'use client';

import { useState, useEffect, useRef, useCallback } from 'react';
import type { WsMessage } from '@/types';

export type WsStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

export interface UseWebSocketResult {
  status: WsStatus;
  lastMessage: WsMessage | null;
}

const RECONNECT_DELAY_MS = 5_000;

export function useWebSocket(url: string): UseWebSocketResult {
  const [status, setStatus] = useState<WsStatus>('disconnected');
  const [lastMessage, setLastMessage] = useState<WsMessage | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  const connect = useCallback(() => {
    if (!mountedRef.current || !url) return;

    setStatus('connecting');

    let ws: WebSocket;
    try {
      ws = new WebSocket(url);
    } catch {
      setStatus('error');
      return;
    }
    wsRef.current = ws;

    ws.onopen = () => {
      if (!mountedRef.current) return;
      setStatus('connected');
    };

    ws.onmessage = (ev: MessageEvent<string>) => {
      if (!mountedRef.current) return;
      try {
        const msg = JSON.parse(ev.data) as WsMessage;
        setLastMessage(msg);
      } catch {
        // ignore malformed frames
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      setStatus('disconnected');
      timerRef.current = setTimeout(connect, RECONNECT_DELAY_MS);
    };

    ws.onerror = () => {
      if (!mountedRef.current) return;
      setStatus('error');
      ws.close();
    };
  }, [url]);

  useEffect(() => {
    mountedRef.current = true;
    connect();
    return () => {
      mountedRef.current = false;
      if (timerRef.current) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, [connect]);

  return { status, lastMessage };
}

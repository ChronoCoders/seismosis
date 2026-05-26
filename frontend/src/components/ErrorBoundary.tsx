'use client';

import React from 'react';

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('ErrorBoundary caught:', error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;
      return (
        <div className="flex items-center justify-center p-8 text-text-muted">
          <div className="text-center">
            <p className="text-sm font-medium text-text-secondary">Bir hata oluştu</p>
            <p className="text-xs mt-1 font-mono">
              {this.state.error?.message ?? 'Bilinmeyen hata'}
            </p>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

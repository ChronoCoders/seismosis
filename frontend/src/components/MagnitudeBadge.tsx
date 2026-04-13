import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

interface MagnitudeBadgeProps {
  magnitude: number;
  size?: 'sm' | 'md' | 'lg' | 'xl';
  showLabel?: boolean;
}

const SIZE_CLASSES = {
  sm: { num: 'text-base font-bold', label: 'text-xs', pad: 'px-2 py-0.5' },
  md: { num: 'text-xl font-bold', label: 'text-xs', pad: 'px-3 py-1' },
  lg: { num: 'text-3xl font-bold', label: 'text-sm', pad: 'px-4 py-2' },
  xl: { num: 'text-5xl font-bold', label: 'text-base', pad: 'px-6 py-3' },
};

export function MagnitudeBadge({
  magnitude,
  size = 'md',
  showLabel = false,
}: MagnitudeBadgeProps) {
  const info = getMagnitudeInfo(magnitude);
  const sz = SIZE_CLASSES[size];

  return (
    <div
      className={`inline-flex flex-col items-center rounded border ${info.bgClass} ${info.borderClass} ${sz.pad}`}
    >
      <span className={`${sz.num} ${info.textClass} leading-none font-mono`}>
        {formatMagnitude(magnitude)}
      </span>
      {showLabel && (
        <span className={`${sz.label} ${info.textClass} uppercase tracking-wider mt-0.5`}>
          {info.label}
        </span>
      )}
    </div>
  );
}

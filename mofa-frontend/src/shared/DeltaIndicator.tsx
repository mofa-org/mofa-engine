import React from 'react';

interface DeltaIndicatorProps {
  value: number;
  isPercent?: boolean;
  className?: string;
}

export function DeltaIndicator({ value, isPercent = true, className }: DeltaIndicatorProps) {
  if (value === 0 || isNaN(value)) return null;

  const isPositive = value > 0;
  const formatted = isPercent 
    ? `${isPositive ? '+' : ''}${value.toFixed(1)}%`
    : `${isPositive ? '+' : ''}${value}`;

  return (
    <span className={`text-[10px] font-mono ${isPositive ? 'text-text-secondary' : 'text-text-dim'} ${className}`}>
      {formatted}
    </span>
  );
}

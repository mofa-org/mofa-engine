import React from 'react';

interface AreaChartProps {
  data: { timestamp: number; value: number }[];
  color?: string;
  gradientId: string;
  height?: number;
  unit?: string;
  formatValue?: (val: number) => string;
}

export function AreaChart({
  data,
  color = '#eab308',
  gradientId,
  height = 100,
  unit = '',
  formatValue = (v) => v.toString()
}: AreaChartProps) {

  if (!data || data.length < 2) {
    return (
      <div 
        style={{ height }} 
        className="w-full flex items-center justify-center text-[11px] font-mono text-text-dim border border-dashed border-border-subtle rounded-md"
      >
        collecting metrics...
      </div>
    );
  }

  const values = data.map(d => d.value);
  const maxVal = Math.max(...values);
  const min = 0;
  const max = maxVal > 0 ? maxVal : 1;
  const range = max;

  const width = 400; // viewBox width
  const chartHeight = height - 20;

  // Build smooth path points matching exact metric peaks
  const points = data.map((d, i) => {
    const x = (i / (data.length - 1)) * width;
    const norm = Math.max(0, Math.min(1, d.value / range));
    const y = chartHeight - norm * (chartHeight - 10) - 5;
    return { x, y };
  });

  const linePath = points.reduce((acc, point, i, a) => {
    if (i === 0) return `M ${point.x},${point.y}`;
    const p0 = a[i - 1];
    const cp1x = p0.x + (point.x - p0.x) / 2;
    const cp1y = p0.y;
    const cp2x = p0.x + (point.x - p0.x) / 2;
    const cp2y = point.y;
    return `${acc} C ${cp1x},${cp1y} ${cp2x},${cp2y} ${point.x},${point.y}`;
  }, '');

  const areaPath = `${linePath} L ${width},${chartHeight} L 0,${chartHeight} Z`;

  const latestVal = values[values.length - 1];

  return (
    <div className="w-full flex flex-col justify-between">
      <div className="flex items-center justify-between text-[11px] font-mono text-text-dim mb-1">
        <span>Min: {formatValue(min)}{unit}</span>
        <span className="font-semibold text-text-primary">Current: {formatValue(latestVal)}{unit}</span>
        <span>Max: {formatValue(max)}{unit}</span>
      </div>

      <svg 
        viewBox={`0 0 ${width} ${chartHeight}`} 
        className="w-full h-auto overflow-visible" 
        style={{ maxHeight: height }}
      >
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.25" />
            <stop offset="100%" stopColor={color} stopOpacity="0.0" />
          </linearGradient>
        </defs>

        {/* Gradient Area Fill */}
        <path d={areaPath} fill={`url(#${gradientId})`} />

        {/* Smooth Line */}
        <path 
          d={linePath} 
          fill="none" 
          stroke={color} 
          strokeWidth="2" 
          strokeLinecap="round" 
        />

        {/* Current Endpoint Dot */}
        {points.length > 0 && (
          <circle
            cx={points[points.length - 1].x}
            cy={points[points.length - 1].y}
            r="3.5"
            fill={color}
            className="animate-ping opacity-75"
          />
        )}
        {points.length > 0 && (
          <circle
            cx={points[points.length - 1].x}
            cy={points[points.length - 1].y}
            r="3"
            fill={color}
          />
        )}
      </svg>
    </div>
  );
}
